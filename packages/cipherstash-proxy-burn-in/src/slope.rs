//! Least-squares growth-rate fitting for memory sample series.
//!
//! A first-to-last difference is dominated by whichever two samples happen to
//! sit at the ends. The fitted slope uses every sample, and R² says how much
//! of the variation is explained by steady growth rather than noise or steps.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fit {
    pub slope_per_second: f64,
    pub intercept: f64,
    /// `None` when every sample has the same value, where R² is undefined.
    pub r_squared: Option<f64>,
    pub samples: usize,
}

/// Fits `y = intercept + slope * x` to `(seconds, bytes)` points.
///
/// Returns `None` when there are fewer than two points or no spread in time.
pub fn fit(points: &[(f64, f64)]) -> Option<Fit> {
    if points.len() < 2 {
        return None;
    }
    let count = points.len() as f64;
    let mean_x = points.iter().map(|(x, _)| x).sum::<f64>() / count;
    let mean_y = points.iter().map(|(_, y)| y).sum::<f64>() / count;
    let mut sum_xx = 0.0;
    let mut sum_xy = 0.0;
    let mut sum_yy = 0.0;
    for (x, y) in points {
        let dx = x - mean_x;
        let dy = y - mean_y;
        sum_xx += dx * dx;
        sum_xy += dx * dy;
        sum_yy += dy * dy;
    }
    if sum_xx == 0.0 {
        return None;
    }
    let slope = sum_xy / sum_xx;
    let r_squared = (sum_yy > 0.0).then(|| (sum_xy * sum_xy) / (sum_xx * sum_yy));
    Some(Fit {
        slope_per_second: slope,
        intercept: mean_y - slope * mean_x,
        r_squared,
        samples: points.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_an_exact_line() {
        let points: Vec<_> = (0..10)
            .map(|second| (f64::from(second), 1_000.0 + 250.0 * f64::from(second)))
            .collect();
        let fit = fit(&points).unwrap();
        assert!((fit.slope_per_second - 250.0).abs() < 1e-9);
        assert!((fit.intercept - 1_000.0).abs() < 1e-9);
        assert!((fit.r_squared.unwrap() - 1.0).abs() < 1e-12);
        assert_eq!(fit.samples, 10);
    }

    #[test]
    fn flat_series_has_zero_slope_and_undefined_r_squared() {
        let points = [(0.0, 42.0), (10.0, 42.0), (20.0, 42.0)];
        let fit = fit(&points).unwrap();
        assert_eq!(fit.slope_per_second, 0.0);
        assert_eq!(fit.r_squared, None);
    }

    #[test]
    fn noise_without_trend_has_low_r_squared() {
        let points = [
            (0.0, 10.0),
            (1.0, 20.0),
            (2.0, 10.0),
            (3.0, 20.0),
            (4.0, 10.0),
        ];
        let fit = fit(&points).unwrap();
        assert_eq!(fit.slope_per_second, 0.0);
        assert_eq!(fit.r_squared, Some(0.0));
    }

    #[test]
    fn needs_two_points_spread_in_time() {
        assert_eq!(fit(&[]), None);
        assert_eq!(fit(&[(1.0, 2.0)]), None);
        assert_eq!(fit(&[(1.0, 2.0), (1.0, 3.0)]), None);
    }
}
