// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

pub(crate) const MIN_WINDOW_SIZE: f64 = 50.0;

pub(crate) struct WindowInput {
    pub weight: f64,
    pub min_size: f64,
    pub max_size: Option<f64>,
    pub fixed_size: Option<f64>,
}

pub(crate) struct WindowOutput {
    pub size: f64,
    #[cfg_attr(not(test), allow(dead_code))]
    pub was_constrained: bool,
}

pub(crate) fn solve_sizes(windows: &[WindowInput], available: f64, gap: f64) -> Vec<WindowOutput> {
    let count = windows.len();
    if count == 0 {
        return vec![];
    }

    let usable = available - gap * (count as f64 - 1.0).max(0.0);

    let total_min: f64 = windows.iter().map(|w| w.min_size).sum();
    if usable <= 0.0 || usable < total_min {
        // There isn't enough room for every window's minimum. Windows that
        // need more than their share keep their minimum, and the rest keep
        // their proportional share, so no window is squeezed below what its
        // app accepts. The caller places the frames and keeps them on screen,
        // letting them overlap.
        let weights: Vec<f64> = windows.iter().map(|w| w.weight.max(0.1)).collect();
        let total_weight: f64 = weights.iter().sum();
        return windows
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let share = usable.max(0.0) * weights[i] / total_weight;
                WindowOutput {
                    size: share.max(w.min_size).max(1.0),
                    was_constrained: true,
                }
            })
            .collect();
    }

    let mut sizes: Vec<f64> = vec![0.0; count];
    let mut fixed = vec![false; count];

    for (i, w) in windows.iter().enumerate() {
        if let Some(fs) = w.fixed_size {
            let max = w.max_size.unwrap_or(f64::MAX);
            sizes[i] = fs.clamp(w.min_size, max);
            fixed[i] = true;
        } else if w.max_size.is_some_and(|m| m <= w.min_size) {
            sizes[i] = w.min_size;
            fixed[i] = true;
        }
    }

    let weights: Vec<f64> = windows.iter().map(|w| w.weight.max(0.1)).collect();

    for _ in 0..count + 1 {
        let used: f64 = (0..count).filter(|&i| fixed[i]).map(|i| sizes[i]).sum();
        let remaining = usable - used;
        let total_weight: f64 = (0..count).filter(|&i| !fixed[i]).map(|i| weights[i]).sum();

        if total_weight <= 0.0 {
            break;
        }

        let mut violated = false;
        for i in 0..count {
            if fixed[i] {
                continue;
            }
            let proposed = remaining * (weights[i] / total_weight);
            if proposed < windows[i].min_size {
                sizes[i] = windows[i].min_size;
                fixed[i] = true;
                violated = true;
                break;
            }
        }

        if !violated {
            for i in 0..count {
                if !fixed[i] {
                    sizes[i] = remaining * (weights[i] / total_weight);
                }
            }
            break;
        }
    }

    let mut excess = 0.0;
    let mut max_fixed = vec![false; count];
    for (i, w) in windows.iter().enumerate() {
        if let Some(max) = w.max_size {
            // A max below the window's minimum can't be met; the minimum wins
            // since the app would refuse the smaller size anyway.
            let max = max.max(w.min_size);
            if sizes[i] > max {
                excess += sizes[i] - max;
                sizes[i] = max;
                max_fixed[i] = true;
            }
        }
    }

    if excess > 0.0 {
        let redist_weight: f64 =
            (0..count).filter(|&i| !max_fixed[i] && !fixed[i]).map(|i| weights[i]).sum();
        if redist_weight > 0.0 {
            for i in 0..count {
                if !max_fixed[i] && !fixed[i] {
                    sizes[i] += excess * (weights[i] / redist_weight);
                }
            }
        }
    }

    for s in sizes.iter_mut() {
        *s = s.max(1.0);
    }

    sizes
        .iter()
        .enumerate()
        .map(|(i, &size)| {
            let w = &windows[i];
            let was_constrained = fixed[i]
                && (Some(size) == w.max_size.map(|m| size.min(m))
                    || (size - w.min_size).abs() < f64::EPSILON);
            WindowOutput { size, was_constrained }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(weight: f64) -> WindowInput {
        WindowInput {
            weight,
            min_size: MIN_WINDOW_SIZE,
            max_size: None,
            fixed_size: None,
        }
    }

    #[test]
    fn empty_input() {
        let result = solve_sizes(&[], 1000.0, 10.0);
        assert!(result.is_empty());
    }

    #[test]
    fn single_window() {
        let result = solve_sizes(&[input(1.0)], 500.0, 10.0);
        assert_eq!(result.len(), 1);
        assert!((result[0].size - 500.0).abs() < 0.01);
    }

    #[test]
    fn equal_weights() {
        let inputs = vec![input(1.0), input(1.0), input(1.0)];
        let result = solve_sizes(&inputs, 1000.0, 10.0);
        let usable = 1000.0 - 20.0;
        let expected = usable / 3.0;
        for r in &result {
            assert!((r.size - expected).abs() < 0.01);
        }
    }

    #[test]
    fn unequal_weights() {
        let inputs = vec![input(1.0), input(2.0)];
        let result = solve_sizes(&inputs, 310.0, 10.0);
        let _usable = 300.0;
        assert!((result[0].size - 100.0).abs() < 0.01);
        assert!((result[1].size - 200.0).abs() < 0.01);
    }

    #[test]
    fn min_violation() {
        let inputs = vec![input(1.0), input(100.0)];
        let result = solve_sizes(&inputs, 160.0, 10.0);
        assert!(result[0].size >= MIN_WINDOW_SIZE);
    }

    /// The window with a large minimum takes its space from the others
    /// instead of being shrunk below what the app will accept.
    #[test]
    fn minimum_constraint_redistributes_to_other_windows() {
        let inputs = vec![
            input(1.0),
            WindowInput {
                weight: 1.0,
                min_size: 700.0,
                max_size: None,
                fixed_size: None,
            },
            input(1.0),
        ];
        let result = solve_sizes(&inputs, 1450.0, 0.0);
        assert!((result[1].size - 700.0).abs() < 0.01);
        assert!((result[0].size - 375.0).abs() < 0.01);
        assert!((result[2].size - 375.0).abs() < 0.01);
    }

    /// When even the minimums don't fit, keep them and let the frames overlap
    /// rather than shrinking every window below its minimum.
    #[test]
    fn minimums_are_kept_when_space_runs_out() {
        let inputs = vec![input(1.0), input(1.0), input(1.0)];
        let result = solve_sizes(&inputs, 100.0, 10.0);
        for r in &result {
            assert!((r.size - MIN_WINDOW_SIZE).abs() < 0.01);
            assert!(r.was_constrained);
        }
    }

    #[test]
    fn max_clamping() {
        let inputs = vec![
            WindowInput {
                weight: 1.0,
                min_size: MIN_WINDOW_SIZE,
                max_size: Some(100.0),
                fixed_size: None,
            },
            input(1.0),
        ];
        let result = solve_sizes(&inputs, 510.0, 10.0);
        assert!(result[0].size <= 100.0);
        assert!((result[0].size + result[1].size - 500.0).abs() < 0.01);
    }

    #[test]
    fn negative_available() {
        let inputs = vec![input(1.0), input(1.0), input(1.0)];
        let result = solve_sizes(&inputs, 10.0, 100.0);
        for r in &result {
            assert!(r.size >= 1.0);
            assert!(r.was_constrained);
        }
    }
}
