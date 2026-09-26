// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Where to put a window to hide it while keeping it on its screen.

use objc2_core_foundation::{CGPoint, CGRect, CGSize};

use crate::sys::geometry::CGRectExt;

/// Returns the origin at which a window of `size` shows exactly 1 point of
/// `target`, in one of its corners. The rest of the window hangs off `target`.
///
/// `target` is the visible frame of the screen the window stays on. `others`
/// are the full bounds of every other display, so a menu bar or Dock on
/// another display counts as part of that display.
///
/// Frames use the top-left origin of the Accessibility API, with y growing
/// down. The corners are tried in the order bottom right, bottom left, top
/// right, top left, and the first one where the window overlaps no other
/// display wins. If every corner overlaps another display, the corner with the
/// smallest overlap wins, and ties go to the earlier corner.
///
/// Bottom corners come first because macOS keeps a window's title bar below
/// the menu bar, so it may refuse to move a window up and off the top edge.
pub fn parking_origin(size: CGSize, target: CGRect, others: &[CGRect]) -> CGPoint {
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
        others.iter().map(|other| other.intersection(&frame).area()).sum()
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

    /// Parks the window and checks that exactly 1 point stays on `target`.
    fn park(size: CGSize, target: CGRect, others: &[CGRect]) -> CGPoint {
        let origin = parking_origin(size, target, others);
        let parked = CGRect { origin, size };
        assert_eq!(1.0, target.intersection(&parked).area());
        origin
    }

    #[test]
    fn h1_one_screen_takes_the_first_corner() {
        let screen = rect(0., 0., 1000., 1000.);
        let origin = park(CGSize::new(400., 300.), screen, &[]);
        assert_eq!(CGPoint::new(999., 999.), origin);
    }

    #[test]
    fn h1_side_by_side_screens_reject_corners_on_the_shared_edge() {
        let left = rect(0., 0., 1000., 1000.);
        let right = rect(1000., 0., 1000., 1000.);
        let size = CGSize::new(400., 300.);
        // The bottom right corner of the left screen touches the right screen.
        assert_eq!(CGPoint::new(-399., 999.), park(size, left, &[right]));
        assert_eq!(CGPoint::new(1999., 999.), park(size, right, &[left]));
    }

    #[test]
    fn h1_screen_stacked_above_another_uses_a_top_corner() {
        let above = rect(0., -1000., 1000., 1000.);
        let below = rect(0., 0., 1000., 1000.);
        let size = CGSize::new(400., 300.);
        assert_eq!(CGPoint::new(999., -1299.), park(size, above, &[below]));
        assert_eq!(CGPoint::new(999., 999.), park(size, below, &[above]));
    }

    #[test]
    fn h1_window_larger_than_the_screen() {
        let size = CGSize::new(3000., 2000.);
        let screen = rect(0., 0., 1000., 1000.);
        assert_eq!(CGPoint::new(999., 999.), park(size, screen, &[]));

        let right = rect(1000., 0., 1000., 1000.);
        assert_eq!(CGPoint::new(-2999., 999.), park(size, screen, &[right]));
    }

    #[test]
    fn h1_every_corner_overlaps_so_the_least_overlapping_wins() {
        let screen = rect(0., 0., 1000., 1000.);
        let others = [
            rect(1000., 0., 1000., 1000.),
            rect(-1000., 0., 1000., 1000.),
            rect(0., 1000., 1000., 1000.),
            // Covers only the right half above the center screen.
            rect(500., -1000., 500., 1000.),
        ];
        // Top left overlaps only the left screen, by 399 x 1 points.
        assert_eq!(
            CGPoint::new(-399., -299.),
            park(CGSize::new(400., 300.), screen, &others)
        );
    }

    #[test]
    fn h1_equal_overlaps_go_to_the_earlier_corner() {
        let screen = rect(0., 0., 1000., 1000.);
        let others = [
            rect(1000., 0., 1000., 1000.),
            rect(-1000., 0., 1000., 1000.),
            // Covers only the right half below the center screen.
            rect(500., 1000., 500., 1000.),
            // Covers only the left half above the center screen.
            rect(0., -1000., 500., 1000.),
        ];
        // Bottom left and top right each overlap a side screen by 399 x 1
        // points. Bottom right and top left also overlap a half screen.
        assert_eq!(
            CGPoint::new(-399., 999.),
            park(CGSize::new(400., 300.), screen, &others)
        );
    }

    #[test]
    fn h1_other_displays_count_with_their_menu_bar() {
        // A display above the main display. Both have a 25-point menu bar.
        let above_visible = rect(0., -1055., 1920., 1055.);
        let main = rect(0., 0., 1920., 1080.);
        // Bottom right would reach 19 points into the main display's menu bar,
        // which its visible frame leaves out.
        assert_eq!(
            CGPoint::new(1919., -1074.),
            park(CGSize::new(400., 20.), above_visible, &[main])
        );
    }

    #[test]
    fn h1_park_on_a_display_at_a_negative_origin() {
        let size = CGSize::new(400., 300.);
        let left = rect(-1920., 0., 1920., 1080.);
        // Bottom right touches the main display, so bottom left wins.
        let main = rect(0., 0., 1000., 1080.);
        assert_eq!(CGPoint::new(-2319., 1079.), park(size, left, &[main]));
        // A shorter main display leaves bottom right clear.
        let short_main = rect(0., 0., 1000., 1000.);
        assert_eq!(CGPoint::new(-1., 1079.), park(size, left, &[short_main]));
    }

    #[test]
    fn h1_fractional_coordinates_stay_exact() {
        let size = CGSize::new(800.25, 600.5);
        let screen = rect(0., 24.5, 1511.5, 957.25);
        assert_eq!(CGPoint::new(1510.5, 980.75), park(size, screen, &[]));

        let right = rect(1511.5, 0., 1000., 1200.);
        assert_eq!(CGPoint::new(-799.25, 980.75), park(size, screen, &[right]));

        let below = rect(0., 981.75, 1511.5, 1000.);
        assert_eq!(CGPoint::new(-799.25, -575.), park(size, screen, &[right, below]));
    }
}
