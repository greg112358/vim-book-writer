/// How far the view has scrolled and where it is heading, in pixels from the top of the text.
/// Scrolling eases toward its target instead of jumping.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Scroll {
    offset: f32,
    target: f32,
}

/// How long the ease takes to cover most of the distance, in seconds.
const EASE: f32 = 0.07;

impl Scroll {
    /// Where the view is now.
    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// Keeps the target, and the position, within the text: from the top to where the last line
    /// can reach the bottom of a view `viewport` tall. Call when either size may have changed.
    pub fn clamp(&mut self, content: f32, viewport: f32) {
        let max = (content - viewport).max(0.0);
        self.target = self.target.clamp(0.0, max);
        self.offset = self.offset.clamp(0.0, max);
    }

    /// Scrolls by `delta` pixels from where it is heading, which is what a wheel does.
    pub fn by(&mut self, delta: f32, content: f32, viewport: f32) {
        self.target += delta;
        self.clamp(content, viewport);
    }

    /// Heads for `position`, which is what putting a line in the middle of the view does.
    pub fn to(&mut self, position: f32, content: f32, viewport: f32) {
        self.target = position;
        self.clamp(content, viewport);
    }

    /// Heads for the least scroll that shows the stretch from `top` to `bottom` with `padding`
    /// to spare above and below. Does nothing if it is already in view.
    pub fn reveal(&mut self, top: f32, bottom: f32, padding: f32, content: f32, viewport: f32) {
        if top - padding < self.target {
            self.target = top - padding;
        } else if bottom + padding > self.target + viewport {
            self.target = bottom + padding - viewport;
        }
        self.clamp(content, viewport);
    }

    /// Moves toward the target for `dt` seconds. True while it is still moving, so the caller
    /// knows to draw another frame.
    pub fn step(&mut self, dt: f32) -> bool {
        let distance = self.target - self.offset;
        if distance.abs() < 0.5 {
            self.offset = self.target;
            return false;
        }
        self.offset += distance * (1.0 - (-dt / EASE).exp());
        true
    }

    /// Arrives at once, for a frame drawn without animation.
    pub fn finish(&mut self) {
        self.offset = self.target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_toward_the_target_and_stops() {
        let mut scroll = Scroll::default();
        scroll.by(1000.0, 5000.0, 600.0);
        assert_eq!(scroll.offset(), 0.0);
        let mut frames = 0;
        let mut last = 0.0;
        while scroll.step(1.0 / 60.0) {
            // Always forward, never past the target.
            assert!(scroll.offset() > last && scroll.offset() < 1000.0);
            last = scroll.offset();
            frames += 1;
            assert!(frames < 200, "never arrived");
        }
        assert_eq!(scroll.offset(), 1000.0);
        // A big jump gets there in about half a second, not instantly and not slowly.
        assert!((10..60).contains(&frames), "{frames} frames");
        assert!(!scroll.step(1.0 / 60.0));
    }

    #[test]
    fn stays_inside_the_text() {
        let mut scroll = Scroll::default();
        scroll.by(-50.0, 2000.0, 600.0);
        scroll.finish();
        assert_eq!(scroll.offset(), 0.0);
        scroll.by(99999.0, 2000.0, 600.0);
        scroll.finish();
        assert_eq!(scroll.offset(), 1400.0);
        // Text shorter than the view doesn't scroll at all, and a growing view pulls it back.
        scroll.clamp(300.0, 600.0);
        assert_eq!(scroll.offset(), 0.0);
    }

    #[test]
    fn reveal_scrolls_as_little_as_it_can() {
        let mut scroll = Scroll::default();
        let reveal = |scroll: &mut Scroll, top, bottom| {
            scroll.reveal(top, bottom, 20.0, 5000.0, 600.0);
            scroll.finish();
            scroll.offset()
        };
        // Already in view: nothing moves.
        assert_eq!(reveal(&mut scroll, 100.0, 127.0), 0.0);
        // Below: just far enough that it and the padding fit at the bottom.
        assert_eq!(reveal(&mut scroll, 700.0, 727.0), 147.0);
        // Still in view after that, so it stays.
        assert_eq!(reveal(&mut scroll, 300.0, 327.0), 147.0);
        // Above: the padding shows over it. Then far below, and back to the very top.
        assert_eq!(reveal(&mut scroll, 100.0, 127.0), 80.0);
        assert_eq!(reveal(&mut scroll, 4000.0, 4027.0), 3447.0);
        assert_eq!(reveal(&mut scroll, 0.0, 27.0), 0.0);
        // The last line can't scroll past the end.
        assert_eq!(reveal(&mut scroll, 4990.0, 5000.0), 4400.0);
    }
}
