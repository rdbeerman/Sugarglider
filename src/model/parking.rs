// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Where to put a window to hide it while keeping it on its screen.

use objc2_core_foundation::{CGPoint, CGRect, CGSize};

use crate::sys::geometry::CGRectExt;

/// Returns the origin at which a window of `size` shows exactly 1 point on
/// `screens[screen]`, in one of that screen's corners. The rest of the window
/// hangs off the screen.
///
/// Frames use the top-left origin of the Accessibility API, with y growing
/// down. The corners are tried in the order bottom right, bottom left, top
/// right, top left, and the first one where the window overlaps no other
/// screen wins. If every corner overlaps another screen, the corner with the
/// smallest overlap wins, and ties go to the earlier corner.
///
/// Bottom corners come first because macOS keeps a window's title bar below
/// the menu bar, so it may refuse to move a window up and off the top edge.
pub fn parking_origin(size: CGSize, screens: &[CGRect], screen: usize) -> CGPoint {
    let target = screens[screen];
    let right = target.max().x - 1.0;
    let bottom = target.max().y - 1.0;
    let left = target.min().x - size.width + 1.0;
    let top = target.min().y - size.height + 1.0;
    let corners = [
        CGPoint::new(right, bottom),
        CGPoint::new(left, bottom),
        CGPoint::new(right, top),
        CGPoint::new(left, top),
    ];
    let overlap_with_others = |origin: CGPoint| -> f64 {
        let frame = CGRect { origin, size };
        screens
            .iter()
            .enumerate()
            .filter(|&(idx, _)| idx != screen)
            .map(|(_, other)| other.intersection(&frame).area())
            .sum()
    };
    // `min_by` returns the first of equal elements, so this is the first corner
    // with no overlap when there is one.
    corners
        .into_iter()
        .map(|origin| (origin, overlap_with_others(origin)))
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(origin, _)| origin)
        .expect("there are four corners")
}

#[cfg(test)]
mod tests {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};

    use super::parking_origin;
    use crate::sys::geometry::CGRectExt;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
        CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
    }

    /// Parks the window and checks that exactly 1 point stays on its screen.
    fn park(size: CGSize, screens: &[CGRect], screen: usize) -> CGPoint {
        let origin = parking_origin(size, screens, screen);
        let parked = CGRect { origin, size };
        assert_eq!(1.0, screens[screen].intersection(&parked).area());
        origin
    }

    #[test]
    fn h1_one_screen_takes_the_first_corner() {
        let screens = [rect(0., 0., 1000., 1000.)];
        let origin = park(CGSize::new(400., 300.), &screens, 0);
        assert_eq!(CGPoint::new(999., 999.), origin);
    }

    #[test]
    fn h1_side_by_side_screens_reject_corners_on_the_shared_edge() {
        let screens = [rect(0., 0., 1000., 1000.), rect(1000., 0., 1000., 1000.)];
        let size = CGSize::new(400., 300.);
        // The bottom right corner of the left screen touches the right screen.
        assert_eq!(CGPoint::new(-399., 999.), park(size, &screens, 0));
        assert_eq!(CGPoint::new(1999., 999.), park(size, &screens, 1));
    }

    #[test]
    fn h1_screen_stacked_above_another_uses_a_top_corner() {
        let screens = [rect(0., -1000., 1000., 1000.), rect(0., 0., 1000., 1000.)];
        let size = CGSize::new(400., 300.);
        assert_eq!(CGPoint::new(999., -1299.), park(size, &screens, 0));
        assert_eq!(CGPoint::new(999., 999.), park(size, &screens, 1));
    }

    #[test]
    fn h1_window_larger_than_the_screen() {
        let size = CGSize::new(3000., 2000.);
        let one_screen = [rect(0., 0., 1000., 1000.)];
        assert_eq!(CGPoint::new(999., 999.), park(size, &one_screen, 0));

        let side_by_side = [rect(0., 0., 1000., 1000.), rect(1000., 0., 1000., 1000.)];
        assert_eq!(CGPoint::new(-2999., 999.), park(size, &side_by_side, 0));
    }

    #[test]
    fn h1_every_corner_overlaps_so_the_least_overlapping_wins() {
        let screens = [
            rect(0., 0., 1000., 1000.),
            rect(1000., 0., 1000., 1000.),
            rect(-1000., 0., 1000., 1000.),
            rect(0., 1000., 1000., 1000.),
            // Covers only the right half above the center screen.
            rect(500., -1000., 500., 1000.),
        ];
        // Top left overlaps only the left screen, by 399 x 1 points.
        assert_eq!(
            CGPoint::new(-399., -299.),
            park(CGSize::new(400., 300.), &screens, 0)
        );
    }
}
