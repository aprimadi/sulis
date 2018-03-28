function on_activate(parent, ability)
  stats = parent:stats()
  if not stats.attack_is_melee then
    return
  end

  targets = parent:targets():hostile():visible()

  targeter = parent:create_targeter(ability)
  targeter:set_show_mouseover(false)
  targeter:set_circle(stats.attack_distance)
  targeter:add_selectable(parent)
  targeter:add_all_effectable(targets)
  targeter:activate()
end

function on_target_select(parent, ability, targets)
  duration = 1.0
  gen = parent:create_anim("whirlwind", duration)
  gen:set_position(gen:param(parent:x() - 2.0), gen:param(parent:y() - 2.0))
  gen:set_particle_size_dist(gen:fixed_dist(4.0), gen:fixed_dist(4.0))
  gen:set_alpha(gen:param(1.0, 0.0, 0.0, -6.0))

  targets = targets:to_table()
  duration_per_target = duration / (#targets + 1)
  for i = 1, #targets do
    cb = ability:create_callback(parent)
	cb:add_target(targets[i])
	cb:set_on_anim_update_fn("attack_target")
    gen:add_callback(cb, duration_per_target * i)
  end
  
  gen:activate()
  ability:activate(parent)
end

function attack_target(parent, ability, targets)
  target = targets:first()

  if target:is_valid() then
    parent:weapon_attack(target)
  end
end

