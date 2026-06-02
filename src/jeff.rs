#[derive(Debug, Clone)]
pub struct Prediction {
    pub predicted: f64,
    pub confidence: f64,
    pub actual: Option<f64>,
    pub divergence: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct DivergenceEvent {
    pub predicted: f64,
    pub actual: f64,
    pub divergence: f64,
    pub tolerance: f64,
}

pub struct JeffPredictor {
    history: Vec<f64>,
    window_size: usize,
    predictions: Vec<Prediction>,
    tolerance: f64,
}

impl JeffPredictor {
    pub fn new(window_size: usize) -> Self {
        Self {
            history: Vec::new(),
            window_size,
            predictions: Vec::new(),
            tolerance: 2.0,
        }
    }

    pub fn with_tolerance(mut self, tolerance: f64) -> Self {
        self.tolerance = tolerance;
        self
    }

    pub fn observe(&mut self, value: f64) {
        self.history.push(value);
    }

    pub fn predict_ahead(&mut self, steps: usize) -> Vec<f64> {
        if self.history.len() < 2 {
            let last = self.history.last().copied().unwrap_or(0.0);
            return vec![last; steps];
        }

        let window: Vec<f64> = self
            .history
            .iter()
            .rev()
            .take(self.window_size)
            .cloned()
            .collect();
        let window = window.into_iter().rev().collect::<Vec<_>>();

        let n = window.len() as f64;
        let sum_x: f64 = (0..window.len()).map(|i| i as f64).sum();
        let sum_y: f64 = window.iter().sum();
        let sum_xy: f64 = window
            .iter()
            .enumerate()
            .map(|(i, v)| i as f64 * v)
            .sum();
        let sum_x2: f64 = (0..window.len()).map(|i| (i * i) as f64).sum();

        let denom = n * sum_x2 - sum_x * sum_x;
        let (slope, intercept) = if denom.abs() < 1e-10 {
            (0.0, sum_y / n)
        } else {
            let slope = (n * sum_xy - sum_x * sum_y) / denom;
            let intercept = (sum_y - slope * sum_x) / n;
            (slope, intercept)
        };

        let start = window.len() as f64;
        let predictions: Vec<f64> = (0..steps)
            .map(|i| intercept + slope * (start + i as f64))
            .collect();

        let residuals: f64 = window
            .iter()
            .enumerate()
            .map(|(i, v)| (v - (intercept + slope * i as f64)).powi(2))
            .sum();
        let mse = residuals / n;
        let confidence = 1.0 / (1.0 + mse).min(1.0);

        for &predicted in &predictions {
            self.predictions.push(Prediction {
                predicted,
                confidence,
                actual: None,
                divergence: None,
            });
        }

        predictions
    }

    pub fn check_divergence(&mut self, actual: f64) -> Option<DivergenceEvent> {
        for pred in self.predictions.iter_mut().rev() {
            if pred.actual.is_none() {
                pred.actual = Some(actual);
                let div = (actual - pred.predicted).abs();
                pred.divergence = Some(div);
                if div > self.tolerance {
                    return Some(DivergenceEvent {
                        predicted: pred.predicted,
                        actual,
                        divergence: div,
                        tolerance: self.tolerance,
                    });
                }
                return None;
            }
        }
        None
    }

    pub fn accuracy(&self) -> f64 {
        let verified: Vec<_> = self
            .predictions
            .iter()
            .filter(|p| p.actual.is_some())
            .collect();
        if verified.is_empty() {
            return 0.0;
        }
        let total_error: f64 = verified
            .iter()
            .map(|p| (p.actual.unwrap() - p.predicted).abs())
            .sum();
        let avg_error = total_error / verified.len() as f64;
        1.0 - (avg_error / self.tolerance).min(1.0)
    }

    pub fn history(&self) -> &[f64] {
        &self.history
    }

    pub fn predictions(&self) -> &[Prediction] {
        &self.predictions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_adds_to_history() {
        let mut j = JeffPredictor::new(10);
        j.observe(1.0);
        j.observe(2.0);
        assert_eq!(j.history(), &[1.0, 2.0]);
    }

    #[test]
    fn predict_linear_trend() {
        let mut j = JeffPredictor::new(10);
        for i in 0..5 {
            j.observe(i as f64 * 2.0);
        }
        let preds = j.predict_ahead(3);
        assert!(preds[0] > 9.0 && preds[0] < 11.0, "predicted {}", preds[0]);
        assert!(preds[1] > 11.0 && preds[1] < 13.0, "predicted {}", preds[1]);
    }

    #[test]
    fn divergence_detected() {
        let mut j = JeffPredictor::new(10).with_tolerance(1.0);
        for i in 0..5 {
            j.observe(i as f64);
        }
        j.predict_ahead(1);
        let event = j.check_divergence(100.0);
        assert!(event.is_some());
        assert!(event.unwrap().divergence > 1.0);
    }

    #[test]
    fn no_divergence_within_tolerance() {
        let mut j = JeffPredictor::new(10).with_tolerance(5.0);
        for i in 0..5 {
            j.observe(i as f64);
        }
        j.predict_ahead(1);
        let event = j.check_divergence(6.0);
        assert!(event.is_none());
    }

    #[test]
    fn accuracy_tracking() {
        let mut j = JeffPredictor::new(10).with_tolerance(10.0);
        for i in 0..10 {
            j.observe(i as f64);
        }
        let preds = j.predict_ahead(5);
        for &pred in &preds {
            j.check_divergence(pred + 0.1);
        }
        let acc = j.accuracy();
        assert!(acc > 0.7, "accuracy was {}", acc);
    }

    #[test]
    fn accuracy_poor_when_diverged() {
        let mut j = JeffPredictor::new(10).with_tolerance(1.0);
        for i in 0..10 {
            j.observe(i as f64);
        }
        let preds = j.predict_ahead(5);
        for &pred in &preds {
            j.check_divergence(pred + 50.0);
        }
        let acc = j.accuracy();
        assert!(acc < 0.5, "accuracy was {}", acc);
    }

    #[test]
    fn predict_with_insufficient_data() {
        let mut j = JeffPredictor::new(10);
        j.observe(42.0);
        let preds = j.predict_ahead(3);
        assert_eq!(preds, vec![42.0, 42.0, 42.0]);
    }

    #[test]
    fn predict_with_empty_history() {
        let mut j = JeffPredictor::new(10);
        let preds = j.predict_ahead(3);
        assert_eq!(preds, vec![0.0, 0.0, 0.0]);
    }
}
