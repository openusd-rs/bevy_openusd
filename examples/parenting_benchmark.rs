//! Compares repeated relationship insertion with retained links and bulk ordering.

use bevy::prelude::*;
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
enum Mode {
    Reinsert,
    Retain,
}

struct Fixture {
    world: World,
    parent: Entity,
    children: Vec<Entity>,
}

impl Fixture {
    fn new(count: usize) -> Self {
        let mut world = World::new();
        let parent = world.spawn_empty().id();
        let children = (0..count)
            .map(|_| world.spawn((Transform::default(), ChildOf(parent))).id())
            .collect();
        Self {
            world,
            parent,
            children,
        }
    }

    fn update(&mut self, mode: Mode, reverse: bool) {
        if reverse {
            self.children.reverse();
        }
        for &child in &self.children {
            let mut entity = self.world.entity_mut(child);
            match mode {
                Mode::Reinsert => {
                    entity.insert((Transform::default(), ChildOf(self.parent)));
                }
                Mode::Retain => {
                    entity.insert(Transform::default());
                    if entity.get::<ChildOf>().map(ChildOf::parent) != Some(self.parent) {
                        entity.insert(ChildOf(self.parent));
                    }
                }
            }
        }
        if matches!(mode, Mode::Retain)
            && !self
                .world
                .get::<Children>(self.parent)
                .unwrap()
                .iter()
                .eq(self.children.iter().copied())
        {
            self.world
                .entity_mut(self.parent)
                .replace_children(&self.children);
        }
    }

    fn verify(&self) {
        assert!(
            self.world
                .get::<Children>(self.parent)
                .unwrap()
                .iter()
                .eq(self.children.iter().copied())
        );
        for &child in &self.children {
            assert_eq!(
                self.world.get::<ChildOf>(child).map(ChildOf::parent),
                Some(self.parent)
            );
            assert_eq!(
                self.world.get::<Transform>(child),
                Some(&Transform::default())
            );
        }
    }
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let count: usize = args
        .first()
        .map_or(Ok(1000), |value| value.parse())
        .unwrap();
    let samples: usize = args.get(1).map_or(Ok(8), |value| value.parse()).unwrap();
    assert!(count > 0 && samples > 0);
    println!("count,reverse,sample,mode,elapsed_ns");
    for reverse in [false, true] {
        let mut fixtures = [Fixture::new(count), Fixture::new(count)];
        for sample in 0..samples + 2 {
            let order = if sample % 2 == 0 { [0, 1] } else { [1, 0] };
            for lane in order {
                let mode = [Mode::Reinsert, Mode::Retain][lane];
                let start = Instant::now();
                fixtures[lane].update(mode, reverse);
                let elapsed = start.elapsed();
                fixtures[lane].verify();
                let phase = if sample < 2 { "warmup" } else { "sample" };
                println!(
                    "{count},{reverse},{phase}_{sample},{mode:?},{}",
                    elapsed.as_nanos()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_and_reinserted_links_have_equal_order() {
        for mode in [Mode::Reinsert, Mode::Retain] {
            let mut fixture = Fixture::new(32);
            for reverse in [false, true, true, false] {
                fixture.update(mode, reverse);
                fixture.verify();
            }
        }
    }
}
