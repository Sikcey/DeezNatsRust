//! Artwork completion must not refresh unrelated retained views or native editors.
use super::{Rect, Scene};
#[derive(Default)]
pub(super) struct Damage {
    pub main: bool,
    pub queue: bool,
    pub rects: Vec<Rect>,
}
pub(super) fn arrival(scene: &Scene, url: &str, bottom: i32) -> Damage {
    matching(scene, |key| key == url, bottom)
}
pub(super) fn all(scene: &Scene, bottom: i32) -> Damage {
    matching(scene, |key| !key.is_empty(), bottom)
}
fn matching(scene: &Scene, wanted: impl Fn(&str) -> bool, bottom: i32) -> Damage {
    let mut damage = Damage::default();
    let mut include = |rect: Rect, main: bool, queue: bool| {
        damage.main |= main;
        damage.queue |= queue;
        let clip = if queue {
            scene.queue_viewport
        } else if main {
            Some(Rect::new(
                rect.x,
                scene.content_top,
                rect.w,
                (bottom - scene.content_top).max(0),
            ))
        } else {
            None
        };
        let rect = if let Some(clip) = clip {
            let x = rect.x.max(clip.x);
            let y = rect.y.max(clip.y);
            Rect::new(
                x,
                y,
                (rect.x + rect.w).min(clip.x + clip.w) - x,
                (rect.y + rect.h).min(clip.y + clip.h) - y,
            )
        } else {
            rect
        };
        if rect.w > 0 && rect.h > 0 {
            damage.rects.push(rect);
        }
    };
    for (i, button) in scene
        .buttons
        .iter()
        .enumerate()
        .filter(|(_, b)| wanted(&b.artwork))
    {
        include(
            button.rect,
            scene.content_buttons.contains(&i),
            scene.queue_buttons.contains(&i),
        );
    }
    for (i, (rect, _)) in scene
        .pictures
        .iter()
        .enumerate()
        .filter(|(_, (_, key))| wanted(key))
    {
        include(
            *rect,
            scene.content_pictures.contains(&i),
            scene.queue_pictures.contains(&i),
        );
    }
    damage
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefetch_completion_does_not_repaint_and_queue_does_not_invalidate_main() {
        let mut scene = Scene::default();
        scene.content_top = 100;
        scene.queue_viewport = Some(Rect::new(700, 100, 250, 400));
        scene.pictures = vec![
            (Rect::new(300, 150, 100, 100), "main".into()),
            (Rect::new(710, 110, 44, 44), "queue".into()),
            (Rect::new(300, 650, 100, 100), "buffered".into()),
        ];
        scene.content_pictures = 0..1;
        scene.queue_pictures = 1..2;
        let prefetch = arrival(&scene, "ahead", 500);
        assert!(!prefetch.main && !prefetch.queue && prefetch.rects.is_empty());
        let queue = arrival(&scene, "queue", 500);
        assert!(queue.queue && !queue.main);
        assert_eq!(queue.rects.len(), 1);
        scene.content_pictures = 2..3;
        let buffered = arrival(&scene, "buffered", 500);
        assert!(buffered.main && buffered.rects.is_empty());
    }
}
