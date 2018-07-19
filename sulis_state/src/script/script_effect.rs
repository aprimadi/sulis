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

use rlua::{Lua, UserData, UserDataMethods};

use sulis_core::util::{ExtInt, Point};
use sulis_rules::{Attribute, BonusKind, BonusList, Damage, DamageKind};

use script::{CallbackData, Result, script_particle_generator, ScriptParticleGenerator,
    script_color_animation, ScriptColorAnimation, ScriptAbility};
use {Effect, GameState};

#[derive(Clone)]
enum Kind {
    Entity(usize),
    Surface(Vec<(i32, i32)>),
}

#[derive(Clone)]
pub struct ScriptEffect {
    kind: Kind,
    name: String,
    tag: String,
    duration: ExtInt,
    deactivate_with_ability: Option<String>,
    pub bonuses: BonusList,
    callbacks: Vec<CallbackData>,
    pgens: Vec<ScriptParticleGenerator>,
    color_anims: Vec<ScriptColorAnimation>,
}

impl ScriptEffect {
    pub fn new_surface(points: Vec<(i32, i32)>, name: &str, duration: ExtInt) -> ScriptEffect {
        ScriptEffect {
            kind: Kind::Surface(points),
            name: name.to_string(),
            tag: "default".to_string(),
            deactivate_with_ability: None,
            duration,
            bonuses: BonusList::default(),
            callbacks: Vec::new(),
            pgens: Vec::new(),
            color_anims: Vec::new(),
        }
    }

    pub fn new_entity(parent: usize, name: &str, duration: ExtInt) -> ScriptEffect {
        ScriptEffect {
            kind: Kind::Entity(parent),
            name: name.to_string(),
            tag: "default".to_string(),
            deactivate_with_ability: None,
            duration,
            bonuses: BonusList::default(),
            callbacks: Vec::new(),
            pgens: Vec::new(),
            color_anims: Vec::new(),
        }
    }
}

impl UserData for ScriptEffect {
    fn add_methods(methods: &mut UserDataMethods<Self>) {
        methods.add_method("apply", |_, effect, _args: ()| {
            apply(effect)
        });
        methods.add_method_mut("add_color_anim", |_, effect, anim: ScriptColorAnimation| {
            effect.color_anims.push(anim);
            Ok(())
        });
        methods.add_method_mut("add_anim", |_, effect, pgen: ScriptParticleGenerator| {
            effect.pgens.push(pgen);
            Ok(())
        });
        methods.add_method_mut("add_num_bonus", &add_num_bonus);
        methods.add_method_mut("add_damage", |_, effect, (min, max, ap): (u32, u32, Option<u32>)| {
            effect.bonuses.add_kind(BonusKind::Damage(Damage { min, max, ap: ap.unwrap_or(0), kind: None }));
            Ok(())
        });
        methods.add_method_mut("add_hidden", |_, effect, ()| {
            effect.bonuses.add_kind(BonusKind::Hidden);
            Ok(())
        });
        methods.add_method_mut("add_move_disabled", |_, effect, ()| {
            effect.bonuses.add_kind(BonusKind::MoveDisabled);
            Ok(())
        });
        methods.add_method_mut("add_attack_disabled", |_, effect, ()| {
            effect.bonuses.add_kind(BonusKind::AttackDisabled);
            Ok(())
        });
        methods.add_method_mut("add_damage_of_kind", |_, effect, (min, max, kind, ap):
                               (u32, u32, String, Option<u32>)| {
            let kind = DamageKind::from_str(&kind);
            effect.bonuses.add_kind(BonusKind::Damage(Damage { min, max, ap: ap.unwrap_or(0), kind: Some(kind) }));
            Ok(())
        });
        methods.add_method_mut("add_armor_of_kind", |_, effect, (value, kind): (i32, String)| {
            let kind = DamageKind::from_str(&kind);
            effect.bonuses.add_kind(BonusKind::ArmorKind { kind, amount: value });
            Ok(())
        });
        methods.add_method_mut("add_attribute_bonus", |_, effect, (attr, amount): (String, i8)| {
            let attribute = match Attribute::from(&attr) {
                None => {
                    warn!("Invalid attribute {} in script", attr);
                    return Ok(());
                }, Some(attr) => attr,
            };
            effect.bonuses.add_kind(BonusKind::Attribute { attribute, amount });
            Ok(())
        });
        methods.add_method_mut("add_callback", |_, effect, cb: CallbackData| {
            effect.callbacks.push(cb);
            Ok(())
        });
        methods.add_method_mut("deactivate_with", |_, effect, ability: ScriptAbility| {
            effect.deactivate_with_ability = Some(ability.id);
            Ok(())
        });
        methods.add_method_mut("set_tag", |_, effect, tag: String| {
            effect.tag = tag;
            Ok(())
        });
    }
}

fn add_num_bonus(_lua: &Lua, effect: &mut ScriptEffect, args: (String, f32)) -> Result<()> {
    let (name, amount) = args;
    let name = name.to_lowercase();
    let amount_int = amount as i32;

    trace!("Adding numeric bonus {} to '{}'", amount, name);
    use sulis_rules::bonus::BonusKind::*;
    match name.as_ref() {
        "armor" => effect.bonuses.add_kind(Armor(amount_int)),
        "ap" => effect.bonuses.add_kind(ActionPoints(amount_int)),
        "reach" => effect.bonuses.add_kind(Reach(amount)),
        "range" => effect.bonuses.add_kind(Range(amount)),
        "initiative" => effect.bonuses.add_kind(Initiative(amount_int)),
        "hit_points" => effect.bonuses.add_kind(HitPoints(amount_int)),
        "accuracy" => effect.bonuses.add_kind(Accuracy(amount_int)),
        "defense" => effect.bonuses.add_kind(Defense(amount_int)),
        "fortitude" => effect.bonuses.add_kind(Fortitude(amount_int)),
        "reflex" => effect.bonuses.add_kind(Reflex(amount_int)),
        "will" => effect.bonuses.add_kind(Will(amount_int)),
        "concealment" => effect.bonuses.add_kind(Concealment(amount_int)),
        "concealment_ignore" => effect.bonuses.add_kind(ConcealmentIgnore(amount_int)),
        "crit_threshold" => effect.bonuses.add_kind(CritThreshold(amount_int)),
        "hit_threshold" => effect.bonuses.add_kind(HitThreshold(amount_int)),
        "graze_threshold" => effect.bonuses.add_kind(GrazeThreshold(amount_int)),
        "graze_multiplier" => effect.bonuses.add_kind(GrazeMultiplier(amount)),
        "hit_multiplier" => effect.bonuses.add_kind(HitMultiplier(amount)),
        "crit_multiplier" => effect.bonuses.add_kind(CritMultiplier(amount)),
        "movement_rate" => effect.bonuses.add_kind(MovementRate(amount)),
        "attack_cost" => effect.bonuses.add_kind(AttackCost(amount_int)),
        _ => warn!("Attempted to add num bonus with invalid type '{}'", name),
    }
    Ok(())
}

const TURNS_TO_MILLIS: u32 = 5000;

fn apply(effect_data: &ScriptEffect) -> Result<()> {
    let mgr = GameState::turn_manager();
    let duration = effect_data.duration * TURNS_TO_MILLIS;

    let mut effect = Effect::new(&effect_data.name, &effect_data.tag, duration, effect_data.bonuses.clone(),
        effect_data.deactivate_with_ability.clone());
    let cbs = effect_data.callbacks.clone();

    let mut marked = Vec::new();
    for anim in effect_data.color_anims.iter() {
        let anim = script_color_animation::create_anim(&anim)?;
        marked.push(anim.get_marked_for_removal());
        GameState::add_animation(anim);
    }

    match &effect_data.kind {
        Kind::Entity(parent) => {
            for pgen in effect_data.pgens.iter() {
                let pgen = script_particle_generator::create_pgen(&pgen, pgen.owned_model())?;
                marked.push(pgen.get_marked_for_removal());
                GameState::add_animation(pgen);
            }

            let entity = mgr.borrow().entity(*parent);
            info!("Apply effect to '{}' with duration {}", entity.borrow().actor.actor.name, duration);
            mgr.borrow_mut().add_effect(effect, &entity, cbs, marked);
        },
        Kind::Surface(points) => {
            let points: Vec<_> = points.iter().map(|(x, y)| Point::new(*x, *y)).collect();
            for pgen in effect_data.pgens.iter() {
                for p in points.iter() {
                    let pgen = script_particle_generator::create_surface_pgen(&pgen, p.x, p.y)?;
                    marked.push(pgen.get_marked_for_removal());
                    GameState::add_animation(pgen);
                }
            }
            let area = GameState::area_state();
            effect.set_surface_for_area(&area.borrow().area.id, &points);
            info!("Add surface to '{}' with duration {}", area.borrow().area.name, duration);
            mgr.borrow_mut().add_surface(effect, &area, points, cbs, marked);
        },
    }

    Ok(())
}
