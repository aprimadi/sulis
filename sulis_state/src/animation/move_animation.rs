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

use std::cmp;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use {animation, AreaState, EntityState};
use sulis_core::ui::Widget;
use sulis_core::util::{self, Point};

pub struct MoveAnimation {
   mover: Rc<RefCell<EntityState>>,
   path: Vec<Point>,
   last_frame_index: i32,
   start_time: Instant,
   frame_time_millis: u32,
   marked_for_removal: bool,
}

impl MoveAnimation {
    pub fn new(mover: Rc<RefCell<EntityState>>,
               path: Vec<Point>, frame_time_millis: u32) -> MoveAnimation {

        MoveAnimation {
            mover,
            path,
            start_time: Instant::now(),
            frame_time_millis,
            marked_for_removal: false,
            last_frame_index: 0, // start at index 0 which is the initial pos
        }
    }

}

impl animation::Animation for MoveAnimation {
    fn update(&mut self, area_state: &mut AreaState, _root: &Rc<RefCell<Widget>>) -> bool {
        if self.marked_for_removal || self.path.is_empty() {
            self.mover.borrow_mut().sub_pos = (0.0, 0.0);
            return false;
        }

        let millis = util::get_elapsed_millis(self.start_time.elapsed());
        let frame_index = cmp::min((millis / self.frame_time_millis) as usize, self.path.len() - 1);
        let frame_frac = (millis % self.frame_time_millis) as f32 / self.frame_time_millis as f32;

        if frame_index != self.path.len() - 1 {
            let frame_delta_x = self.path[frame_index + 1].x - self.path[frame_index].x;
            let frame_delta_y = self.path[frame_index + 1].y - self.path[frame_index].y;
            self.mover.borrow_mut().sub_pos = (frame_delta_x as f32 * frame_frac,
                                               frame_delta_y as f32 * frame_frac);
        }

        if frame_index as i32 == self.last_frame_index {
            return true;
        }
        let move_ap = frame_index as i32 - self.last_frame_index;
        self.last_frame_index = frame_index as i32;

        let p = self.path[frame_index];
        if !area_state.move_entity(&self.mover, p.x, p.y, move_ap as u32) {
            return false;
        }

        trace!("Updated move animation at frame {}", frame_index);
        if frame_index == self.path.len() - 1 {
            self.mover.borrow_mut().sub_pos = (0.0, 0.0);
            return false;
        }


        true
    }

    fn check(&mut self, entity: &Rc<RefCell<EntityState>>) {
        let exiting = self.mover.borrow().index == entity.borrow().index;

        if exiting {
            self.marked_for_removal = true;
            self.mover.borrow_mut().sub_pos = (0.0, 0.0);
        }
    }

    fn get_owner(&self) -> &Rc<RefCell<EntityState>> {
        &self.mover
    }
}
