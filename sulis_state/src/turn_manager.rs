//  This file is part of Sulis, a turn based RPG written in Rust.
//  Copyright 2018 Jared Stephen
//
//  Sulis is free software: you can redistribute it and/or modify
//  it under the terms of the GNU General Public License as published by
//  the Free Software Foundation, either version 3 of the License, or
//  (at your option) any later version.
//
//  Sulis is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU General Public License for more details.
//
//  You should have received a copy of the GNU General Public License
//  along with Sulis.  If not, see <http://www.gnu.org/licenses/>

use std::rc::Rc;
use std::cell::{RefCell, Cell};
use std::collections::{HashSet, HashMap, VecDeque, vec_deque::Iter};

use rand::{self, Rng};

use sulis_core::util::Point;
use sulis_module::{Faction, Module};
use crate::script::{CallbackData};
use crate::{AreaState, ChangeListener, ChangeListenerList, Effect, EntityState, GameState};

pub const ROUND_TIME_MILLIS: u32 = 5000;

#[derive(Clone, Copy)]
enum Entry {
    Entity(usize),
    Effect(usize),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct EncounterRef {
    area_id: String,
    encounter_index: usize,
}

pub struct TurnManager {
    entities: Vec<Option<Rc<RefCell<EntityState>>>>,
    pub(crate) effects: Vec<Option<Effect>>,
    surfaces: Vec<usize>,
    effects_remove_next_update: Vec<usize>,
    entities_move_callback_next_update: HashSet<usize>,
    combat_active: bool,

    pub listeners: ChangeListenerList<TurnManager>,
    order: VecDeque<Entry>,

    pub(crate) ai_groups: HashMap<usize, EncounterRef>,
    pub(crate) cur_ai_group_index: usize,
}

impl Default for TurnManager {
    fn default() -> TurnManager {
        TurnManager {
            entities: Vec::new(),
            effects: Vec::new(),
            surfaces: Vec::new(),
            effects_remove_next_update: Vec::new(),
            entities_move_callback_next_update: HashSet::new(),
            listeners: ChangeListenerList::default(),
            order: VecDeque::new(),
            combat_active: false,
            ai_groups: HashMap::new(),
            cur_ai_group_index: 0,
        }
    }
}

impl TurnManager {
    pub(crate) fn clear(&mut self) {
        self.entities.clear();
        self.effects.clear();
        self.surfaces.clear();
        self.effects_remove_next_update.clear();
        self.combat_active = false;
        self.listeners = ChangeListenerList::default();
        self.order.clear();
        self.cur_ai_group_index = 0;
        self.ai_groups.clear();
    }

    pub fn effect_mut_checked(&mut self, index: usize) -> Option<&mut Effect> {
        if index >= self.effects.len() { return None; }

        self.effects[index].as_mut()
    }

    pub fn effect_mut(&mut self, index: usize) -> &mut Effect {
        self.effects[index].as_mut().unwrap()
    }

    pub fn effect(&self, index: usize) -> &Effect {
        self.effects[index].as_ref().unwrap()
    }

    pub fn effect_checked(&self, index: usize) -> Option<&Effect> {
        if index >= self.effects.len() { return None; }

        self.effects[index].as_ref()
    }

    pub fn effect_iter(&self) -> EffectIterator {
        EffectIterator { mgr: &self, index: 0 }
    }

    pub fn active_iter(&self) -> ActiveEntityIterator {
        ActiveEntityIterator { mgr: &self, entry_iter: self.order.iter() }
    }

    pub fn entity_iter(&self) -> EntityIterator {
        EntityIterator { mgr: &self, index: 0 }
    }

    pub fn has_entity(&self, index: usize) -> bool {
        if index >= self.entities.len() { return false; }

        self.entities[index].is_some()
    }

    pub fn get_next_ai_group(&mut self, area_id: &str, enc_index: usize) -> usize {
        let value = self.cur_ai_group_index;

        self.cur_ai_group_index += 1;
        self.ai_groups.insert(value, EncounterRef {
            area_id: area_id.to_string(),
            encounter_index: enc_index,
        });
        value
    }

    pub fn entity_checked(&self, index: usize) -> Option<Rc<RefCell<EntityState>>> {
        if index >= self.entities.len() { return None; }
        self.entities[index].clone()
    }

    pub fn entity(&self, index: usize) -> Rc<RefCell<EntityState>> {
        Rc::clone(self.entities[index].as_ref().unwrap())
    }

    #[must_use]
    pub fn update_on_moved_in_surface(&mut self) -> Vec<(Rc<CallbackData>, usize)> {
        let mut result = Vec::new();

        for index in self.surfaces.iter() {
            // can't use the method because of borrow checker
            let effect = self.effects[*index].as_mut().unwrap();

            result.append(&mut effect.update_on_moved_in_surface());
        }

        result
    }

    #[must_use]
    pub fn update_entity_move_callbacks(&mut self) -> Vec<Rc<CallbackData>> {
        let mut cbs = Vec::new();

        let indices: Vec<_> = self.entities_move_callback_next_update.drain().collect();
        for index in indices {
            let entity = self.entity(index);
            cbs.append(&mut entity.borrow().callbacks(&self));
        }

        cbs
    }

    #[must_use]
    pub fn update(&mut self, elapsed_millis: u32) -> (Vec<Rc<CallbackData>>, Vec<Rc<CallbackData>>) {
        // need to do an additional copy to satisfy the borrow checker here
        let to_remove: Vec<usize> = self.effects_remove_next_update.drain(..).collect();

        let mut removal_cbs = Vec::new();
        for index in to_remove {
            removal_cbs.append(&mut self.remove_effect(index));
        }

        let mut turn_cbs = Vec::new();
        let elapsed_millis = if !self.combat_active { elapsed_millis } else { 0 };

        // removal just replaces some with none, so we can safely iterate
        for index in 0..self.effects.len() {
            let (is_removal, mut effect_cbs) = self.update_effect(index, elapsed_millis);
            turn_cbs.append(&mut effect_cbs);
            if is_removal {
                self.queue_remove_effect(index);
            }
        }

        for index in 0..self.entities.len() {
            if self.update_entity(index, elapsed_millis) {
                self.remove_entity(index);
            }
        }

        (turn_cbs, removal_cbs)
    }

    #[must_use]
    fn update_effect(&mut self, index: usize, elapsed_millis: u32) -> (bool, Vec<Rc<CallbackData>>) {
        let effect = match self.effects[index] {
            None => return (false, Vec::new()),
            Some(ref mut effect) => effect,
        };

        let cbs = effect.update(elapsed_millis);
        (effect.is_removal(), cbs)
    }

    fn update_entity(&mut self, index: usize, elapsed_millis: u32) -> bool {
        let entity = match self.entities[index].as_ref() {
            None => return false,
            Some(entity) => entity,
        };

        let mut entity = entity.borrow_mut();
        entity.actor.elapse_time(elapsed_millis, &self.effects);
        entity.is_marked_for_removal()
    }

    #[must_use]
    pub fn next(&mut self) -> Vec<Rc<CallbackData>> {
        let cbs = self.iterate_to_next_entity();
        self.init_turn_for_current_entity();

        match self.current() {
            Some(entity) => {
                if entity.borrow().is_party_member() {
                    GameState::set_selected_party_member(entity);
                } else {
                    GameState::clear_selected_party_member();
                }
            }, None => unreachable!(),
        }

        self.listeners.notify(&self);
        cbs
    }

    fn init_turn_for_current_entity(&mut self) {
        let current = match self.order.front() {
            Some(Entry::Entity(index)) => {
                match self.entities[*index] {
                    None => unreachable!(),
                    Some(ref entity) => entity,
                }
            },
            _ => unreachable!(),
        };

        if current.borrow().is_party_member() {
            GameState::set_selected_party_member(Rc::clone(current));
        }

        let mut current = current.borrow_mut();
        current.actor.init_turn();
        current.actor.elapse_time(ROUND_TIME_MILLIS, &self.effects);

        debug!("'{}' now has the active turn", current.actor.actor.name);
    }

    pub fn current(&self) -> Option<Rc<RefCell<EntityState>>> {
        if !self.combat_active { return None; }

        match self.order.front() {
            Some(Entry::Entity(index)) => {
                match self.entities[*index] {
                    None => unreachable!(),
                    Some(ref entity) => Some(Rc::clone(entity)),
                }
            },
            _ => None,
        }
    }

    #[must_use]
    fn iterate_to_next_entity(&mut self) -> Vec<Rc<CallbackData>> {
        let mut cbs = Vec::new();
        let mut current_ended = false;

        loop {
            if current_ended && self.current_is_active_entity() { break; }

            let front = match self.order.pop_front() {
                None => unreachable!(),
                Some(entry) => entry,
            };

            match front {
                Entry::Effect(index) => {
                    let (removal, mut effect_cbs) = self.update_effect(index, ROUND_TIME_MILLIS);
                    cbs.append(&mut effect_cbs);
                    if removal { self.queue_remove_effect(index); }
                    else { self.order.push_back(Entry::Effect(index)); }
                },
                Entry::Entity(index) => {
                    if let Some(entity) = &self.entities[index] {
                        entity.borrow_mut().actor.end_turn();
                    }

                    self.order.push_back(Entry::Entity(index));
                    current_ended = true;
                }
            }
        }

        cbs
    }

    fn current_is_active_entity(&self) -> bool {
        if let Some(Entry::Entity(index)) = self.order.front() {
            if let Some(entity) = &self.entities[*index] {
                let entity = entity.borrow();
                return entity.is_party_member() || entity.is_ai_active();
            }
        }

        false
    }

    pub fn check_ai_activation_for_party(&mut self, area_state: &mut AreaState) {
        for entity in GameState::party() {
            self.check_ai_activation(&entity, area_state);
        }
    }

    pub fn check_ai_activation(&mut self, mover: &Rc<RefCell<EntityState>>, area_state: &mut AreaState) {
        if mover.borrow().actor.stats.hidden { return; }

        let mut groups_to_activate: HashSet<usize> = HashSet::new();
        let mut state_changed = false;

        for entity in self.entities.iter() {
            let entity = match entity {
                None => continue,
                Some(ref entity) => entity,
            };

            if Rc::ptr_eq(mover, entity) { continue; }

            let mut entity = entity.borrow_mut();
            if entity.actor.is_dead() { continue; }
            if !entity.is_hostile(mover) { continue; }
            if !entity.location.is_in(&area_state) { continue; }
            if entity.actor.actor.ai.is_none() && !entity.is_party_member() { continue; }

            let mover = mover.borrow();
            if !area_state.has_visibility(&mover, &entity) && !area_state.has_visibility(&entity, &mover) {
                continue;
            }

            self.activate_entity_ai(&mut entity, &mut groups_to_activate);
            state_changed = true;
        }

        if !state_changed { return; }

        self.activate_entity_ai(&mut mover.borrow_mut(), &mut groups_to_activate);

        for entity in self.entities.iter() {
            let entity = match entity {
                None => continue,
                Some(ref entity) => entity,
            };

            let mut entity = entity.borrow_mut();
            if entity.is_ai_active() { continue; }
            if !entity.location.is_in(&area_state) { continue; }

            match entity.ai_group() {
                None => continue,
                Some(group) => {
                    if groups_to_activate.contains(&group) {
                        entity.set_ai_active(true);
                    }
                }
            }
        }

        for group in groups_to_activate {
            let enc_ref = self.ai_groups.get(&group).unwrap().clone();
            if enc_ref.area_id == area_state.area.id {
                area_state.fire_on_encounter_activated(enc_ref.encounter_index, &mover);
            } else {
                let area_state = GameState::get_area_state(&enc_ref.area_id).unwrap();
                area_state.borrow_mut().fire_on_encounter_activated(enc_ref.encounter_index, &mover);
            }
        }

        if !self.combat_active {
            self.set_combat_active(true);
            loop {
                if self.current_is_active_entity() { break; }
                let front = self.order.pop_front().unwrap();
                self.order.push_back(front);
            }
            area_state.bump_party_overlap(self);
            self.init_turn_for_current_entity();
        }

        self.listeners.notify(&self);
    }

    fn activate_entity_ai(&self, entity: &mut EntityState, groups: &mut HashSet<usize>) {
        if entity.is_party_member() { return; }
        if entity.is_ai_active() { return; }

        trace!("Activate AI for {}", entity.actor.actor.name);
        entity.set_ai_active(true);

        if let Some(group) = entity.ai_group() {
            groups.insert(group);
        }
    }

    pub fn is_combat_active(&self) -> bool {
        self.combat_active
    }

    fn set_combat_active(&mut self, active: bool) {
        if active == self.combat_active { return; }

        info!("Setting combat mode active = {}", active);
        self.combat_active = active;

        if !active {
            self.end_combat();
        } else {
            self.initiate_combat();
        }
    }

    fn end_combat(&mut self) {
        for entity in self.entities.iter() {
            let entity = match entity {
                None => continue,
                Some(ref entity) => entity,
            };
            let mut entity = entity.borrow_mut();

            entity.set_ai_active(false);

            if !entity.is_party_member() { continue; }

            entity.actor.end_encounter();
        }

        if GameState::selected().is_empty() {
            GameState::set_selected_party_member(GameState::player());
        }
    }

    fn initiate_combat(&mut self) {
        // first, compute initiative for each entry in the list
        let initiative_roll_max = Module::rules().initiative_roll_max;
        let mut initiative = vec![0; self.order.len()];
        let mut index = initiative.len();
        let mut last_initiative = 0;
        for entry in self.order.iter().rev() {
            index -= 1;
            match entry {
                Entry::Entity(entity_index) => {
                    let base = self.entities[*entity_index].as_ref()
                        .unwrap().borrow().actor.stats.initiative;
                    last_initiative = base + rand::thread_rng().gen_range(0, initiative_roll_max);
                    initiative[index] = 2* last_initiative;
                },
                Entry::Effect(_) => {
                    // this effect should come just before the associated entity
                    initiative[index] = 2 * last_initiative - 1;
                }
            }
        }


        let mut entries: Vec<_> = self.order.drain(..).zip(initiative).collect();
        entries.sort_by_key(|&(_, initiative)| { initiative });
        entries.into_iter().for_each(|(entry, _)| self.order.push_front(entry));

        for entity in self.entities.iter() {
            let entity = match entity {
                None => continue,
                Some(ref entity) => entity,
            };

            entity.borrow_mut().actor.end_turn();
            entity.borrow_mut().actor.set_overflow_ap(0);
        }
        GameState::set_clear_anims();
    }

    pub(crate) fn fire_on_moved_next_update(&mut self, entity_index: usize) {
        self.entities_move_callback_next_update.insert(entity_index);
    }

    pub(crate) fn increment_surface_squares_moved(&mut self, entity_index: usize, surface_index: usize) {
        let surface = self.effect_mut(surface_index);
        surface.increment_squares_moved(entity_index);
    }

    pub (crate) fn add_to_surface(&mut self, entity_index: usize, surface_index: usize) {
        let entity = self.entity(entity_index);
        let surface = self.effect_mut(surface_index);
        info!("Add '{}' from surface {}", entity.borrow().actor.actor.name, surface_index);
        entity.borrow_mut().actor.add_effect(surface_index, surface.bonuses().clone());
        surface.increment_squares_moved(entity_index);
    }

    pub (crate) fn remove_from_surface(&mut self, entity_index: usize, surface_index: usize) {
        let entity = match self.entity_checked(entity_index) {
            None => return,
            Some(entity) => entity,
        };
        assert!(self.effects[surface_index].is_some());
        info!("Remove '{}' from surface {}", entity.borrow().actor.actor.name, surface_index);
        entity.borrow_mut().actor.remove_effect(surface_index);
    }

    pub fn readd_entity(&mut self, entity: &Rc<RefCell<EntityState>>) {
        let index = entity.borrow().index();
        self.order.push_back(Entry::Entity(index));
    }

    pub fn add_entity(&mut self, entity: &Rc<RefCell<EntityState>>, is_dead: bool) -> usize {
        {
            let entity = entity.borrow();
            let uid = entity.unique_id();
            for other_entity in self.entity_iter() {
                if uid == other_entity.borrow().unique_id() {
                    warn!("Adding entity with duplicate unique ID '{}', this could cause script issues",
                          uid);
                    break;
                }
            }
        }

        let entity_to_add = Rc::clone(entity);
        self.entities.push(Some(entity_to_add));
        let index = self.entities.len() - 1;

        if !is_dead {
            self.order.push_back(Entry::Entity(index));
            debug!("Added entity with unique id '{}' at {} to turn timer", entity.borrow().unique_id(), index);
        }

        entity.borrow_mut().set_index(index);
        entity.borrow_mut().actor.init_turn();
        self.listeners.notify(&self);

        index
    }

    fn add_effect_internal(&mut self, mut effect: Effect, cbs: Vec<CallbackData>,
                           removal_markers: Vec<Rc<Cell<bool>>>) -> usize {
        effect.removal_listeners.add(ChangeListener::new("anim", Box::new(move |_| {
            removal_markers.iter().for_each(|m| m.set(true));
        })));

        let index = self.effects.len();
        for mut cb in cbs {
            cb.set_effect(index);
            effect.add_callback(Rc::new(cb));
        }

        self.effects.push(Some(effect));
        self.order.push_back(Entry::Effect(index));
        debug!("Added effect at {} to turn manager", index);

        index
    }

    /// Returns the index that will be set for the next effect that is added
    /// to this turn manager
    pub fn get_next_effect_index(&self) -> usize {
        self.effects.len()
    }

    pub fn add_surface(&mut self, effect: Effect, area_state: &Rc<RefCell<AreaState>>,
                       points: Vec<Point>, cbs: Vec<CallbackData>,
                       removal_markers: Vec<Rc<Cell<bool>>>) -> usize {
        let index = self.add_effect_internal(effect, cbs, removal_markers);
        self.surfaces.push(index);
        let entities = area_state.borrow_mut().add_surface(index, points);

        for entity in entities {
            self.add_to_surface(entity, index);
        }

        index
    }

    pub fn add_effect(&mut self, effect: Effect, entity: &Rc<RefCell<EntityState>>,
                      cbs: Vec<CallbackData>, removal_markers: Vec<Rc<Cell<bool>>>) -> usize {
        let index = self.add_effect_internal(effect, cbs, removal_markers);

        let bonuses = self.effect(index).bonuses().clone();
        entity.borrow_mut().actor.add_effect(index, bonuses);

        index
    }

    /// Adds the specified cells to be set to true when the given effect is removed.  this
    /// is used when loading, in order to associate animations with effects
    pub fn add_removal_listener_for_effect(&mut self, index: usize, marked: Vec<Rc<Cell<bool>>>) {
        match self.effects.get_mut(index) {
            None => unreachable!(),
            Some(ref mut effect) => match effect {
                None => unreachable!(),
                Some(ref mut effect) => {
                    effect.removal_listeners.add(ChangeListener::new("anim", Box::new(move |_| {
                        marked.iter().for_each(|m| m.set(true));
                    })));
                }
            }
        }
    }

    // queue up the effect removal for later because we want to
    // call the callbacks before removal, and we must call them
    // outside the turn manager to avoid double borrow errors
    fn queue_remove_effect(&mut self, index: usize) {
        self.effects_remove_next_update.push(index);
    }

    fn remove_effect(&mut self, index: usize) -> Vec<Rc<CallbackData>> {
        let cbs;
        let mut entities = HashSet::new();
        if let Some(effect) = &self.effects[index] {
            if let Some((ref area_id, ref points)) = effect.surface() {
                let area = GameState::get_area_state(area_id).unwrap();
                entities = area.borrow_mut().remove_surface(index, points);
            }

            cbs = effect.callbacks.clone();
        } else {
            cbs = Vec::new();
        }

        for entity in entities {
            self.remove_from_surface(entity, index);
        }
        self.effects[index] = None;
        self.order.retain(|e| {
            match e {
                Entry::Effect(i) => *i != index,
                Entry::Entity(_) => true,
            }
        });

        self.surfaces.retain(|e| *e != index);

        cbs
    }

    fn check_encounter_cleared(&self, entity: &Rc<RefCell<EntityState>>) -> Option<usize>{
        let ai_group = match entity.borrow().ai_group() {
            None => return None,
            Some(index) => index,
        };

        debug!("Check encounter cleared: {}", ai_group);
        for other in self.entity_iter() {
            let other = other.borrow();
            if other.actor.hp() <= 0 { continue; }
            if let Some(index) = other.ai_group() {
                if index == ai_group {
                    debug!("  Found blocking entity '{}' with {}",
                           other.actor.actor.id, other.actor.hp());
                    return None;
                }
            }
        }

        Some(ai_group)
    }

    fn remove_entity(&mut self, index: usize) {
        let entity = Rc::clone(self.entities[index].as_ref().unwrap());
        let area_state = GameState::get_area_state(&entity.borrow().location.area_id).unwrap();
        let surfaces = area_state.borrow_mut().remove_entity(&entity, &self);

        for surface in surfaces.iter() {
            self.remove_from_surface(index, *surface);
        }

        let cur_hp = entity.borrow().actor.hp();
        if cur_hp > 0 {
            // don't want all the entity checks, just to set the value
            // to zero
            entity.borrow_mut().actor.remove_hp(cur_hp as u32);
        }
        // don't actually remove the entity from the backing vec, to allow
        // scripts to continue to reference it
        // self.entities[index] = None;
        entity.borrow_mut().marked_for_removal = false;

        // can't do this with a collect because of lifetime issues
        let mut effects_to_remove = Vec::new();
        {
            let entity = entity.borrow();
            for index in entity.actor.effects_iter() {
                effects_to_remove.push(*index);
                self.queue_remove_effect(*index);
            }
        }

        self.order.retain(|e| {
            match e {
                Entry::Entity(i) => *i != index,
                Entry::Effect(i) => !effects_to_remove.contains(i),
            }
        });

        if self.order.iter().all(|e| {
            match e {
                Entry::Effect(_) => true,
                Entry::Entity(index) => {
                    let entity = self.entities[*index].as_ref().unwrap().borrow();
                    !entity.is_ai_active() || entity.actor.faction() != Faction::Hostile
                }
            }
        }) {
            self.set_combat_active(false);
        }

        if let Some(ai_group) = self.check_encounter_cleared(&entity) {
            let enc_ref = self.ai_groups.get(&ai_group).unwrap().clone();
            let area_state = GameState::get_area_state(&enc_ref.area_id).unwrap();
            area_state.borrow_mut().fire_on_encounter_cleared(enc_ref.encounter_index, &entity);
        }

        self.listeners.notify(&self);
    }
}

pub struct ActiveEntityIterator<'a> {
    entry_iter: Iter<'a, Entry>,
    mgr: &'a TurnManager,
}

impl<'a> Iterator for ActiveEntityIterator<'a> {
    type Item = &'a Rc<RefCell<EntityState>>;
    fn next(&mut self) -> Option<&'a Rc<RefCell<EntityState>>> {
        if !self.mgr.is_combat_active() { return None; }

        loop {
            match self.entry_iter.next() {
                None => return None,
                Some(ref entry) => match entry {
                    Entry::Effect(_) => (),
                    Entry::Entity(index) => {
                        let entity = self.mgr.entities[*index].as_ref().unwrap();
                        if entity.borrow().is_party_member() || entity.borrow().is_ai_active() {
                            return Some(entity);
                        }
                    }
                }
            }
        }
    }
}
pub struct EntityIterator<'a> {
    mgr: &'a TurnManager,
    index: usize,
}

impl<'a> Iterator for EntityIterator<'a> {
    type Item = Rc<RefCell<EntityState>>;
    fn next(&mut self) -> Option<Rc<RefCell<EntityState>>> {
        loop {
            let next = self.mgr.entities.get(self.index);

            self.index += 1;

            match next {
                None => return None,
                Some(e) => match e {
                    &None => continue,
                    &Some(ref entity) => return Some(Rc::clone(entity))
                }
            }
        }
    }
}

pub struct EffectIterator<'a> {
    mgr: &'a TurnManager,
    index: usize,
}

impl<'a> Iterator for EffectIterator<'a> {
    type Item = &'a Effect;
    fn next(&mut self) -> Option<&'a Effect> {
        loop {
            let next = self.mgr.effects.get(self.index);

            self.index += 1;

            match &next {
                None => return None,
                Some(e) => match e {
                    None => continue,
                    Some(e) => return Some(e)
                }
            };
        }
    }
}
