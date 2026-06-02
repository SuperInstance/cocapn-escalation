use chrono::{DateTime, Utc};
use cocapn_core::DeviceTier;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BudgetError {
    #[error("over budget: spent {spent:.2}, limit {limit:.2}")]
    OverBudget { spent: f64, limit: f64 },
    #[error("budget is frozen")]
    Frozen,
}

#[derive(Debug, Clone)]
pub struct Transaction {
    pub timestamp: DateTime<Utc>,
    pub amount: f64,
    pub tier: DeviceTier,
    pub reason: String,
    pub device: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BudgetState {
    UnderBudget,
    ApproachingLimit(f64),
    OverBudget,
    Frozen,
}

#[derive(Debug, Clone)]
pub struct BudgetReport {
    pub spent_today: f64,
    pub spent_this_month: f64,
    pub daily_remaining: f64,
    pub monthly_remaining: f64,
    pub transaction_count: usize,
}

pub struct Budget {
    daily_limit: f64,
    monthly_limit: f64,
    spent_today: f64,
    spent_this_month: f64,
    transactions: Vec<Transaction>,
    frozen: bool,
}

impl Budget {
    pub fn new(daily_limit: f64, monthly_limit: f64) -> Self {
        Self {
            daily_limit,
            monthly_limit,
            spent_today: 0.0,
            spent_this_month: 0.0,
            transactions: Vec::new(),
            frozen: false,
        }
    }

    pub fn spend(
        &mut self,
        amount: f64,
        tier: DeviceTier,
        reason: String,
    ) -> Result<BudgetState, BudgetError> {
        if self.frozen {
            return Err(BudgetError::Frozen);
        }

        if self.spent_today + amount > self.daily_limit {
            return Err(BudgetError::OverBudget {
                spent: self.spent_today + amount,
                limit: self.daily_limit,
            });
        }

        self.spent_today += amount;
        self.spent_this_month += amount;

        let txn = Transaction {
            timestamp: Utc::now(),
            amount,
            tier,
            reason,
            device: String::new(),
        };
        self.transactions.push(txn);

        let usage = self.spent_today / self.daily_limit;
        if usage >= 1.0 {
            Ok(BudgetState::OverBudget)
        } else if usage >= 0.8 {
            Ok(BudgetState::ApproachingLimit(usage))
        } else {
            Ok(BudgetState::UnderBudget)
        }
    }

    pub fn remaining_daily(&self) -> f64 {
        (self.daily_limit - self.spent_today).max(0.0)
    }

    pub fn remaining_monthly(&self) -> f64 {
        (self.monthly_limit - self.spent_this_month).max(0.0)
    }

    pub fn can_afford(&self, amount: f64) -> bool {
        !self.frozen && self.spent_today + amount <= self.daily_limit
    }

    pub fn daily_report(&self) -> BudgetReport {
        BudgetReport {
            spent_today: self.spent_today,
            spent_this_month: self.spent_this_month,
            daily_remaining: self.remaining_daily(),
            monthly_remaining: self.remaining_monthly(),
            transaction_count: self.transactions.len(),
        }
    }

    pub fn reset_daily(&mut self) {
        self.spent_today = 0.0;
    }

    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    pub fn unfreeze(&mut self) {
        self.frozen = false;
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    pub fn transactions(&self) -> &[Transaction] {
        &self.transactions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_budget_has_full_limits() {
        let b = Budget::new(10.0, 100.0);
        assert_eq!(b.remaining_daily(), 10.0);
        assert_eq!(b.remaining_monthly(), 100.0);
    }

    #[test]
    fn spend_tracks_amount() {
        let mut b = Budget::new(10.0, 100.0);
        let state = b.spend(2.0, DeviceTier::Cloud, "inference".into()).unwrap();
        assert_eq!(state, BudgetState::UnderBudget);
        assert_eq!(b.remaining_daily(), 8.0);
    }

    #[test]
    fn spend_approaching_limit() {
        let mut b = Budget::new(10.0, 100.0);
        let state = b.spend(8.5, DeviceTier::Cloud, "big job".into()).unwrap();
        assert!(matches!(state, BudgetState::ApproachingLimit(r) if r >= 0.8));
    }

    #[test]
    fn spend_over_budget_blocked() {
        let mut b = Budget::new(10.0, 100.0);
        b.spend(5.0, DeviceTier::Cloud, "first".into()).unwrap();
        let err = b.spend(6.0, DeviceTier::Cloud, "second".into()).unwrap_err();
        assert!(matches!(err, BudgetError::OverBudget { .. }));
    }

    #[test]
    fn can_afford_within_limit() {
        let b = Budget::new(10.0, 100.0);
        assert!(b.can_afford(5.0));
        assert!(!b.can_afford(15.0));
    }

    #[test]
    fn daily_reset() {
        let mut b = Budget::new(10.0, 100.0);
        b.spend(5.0, DeviceTier::Cloud, "job".into()).unwrap();
        b.reset_daily();
        assert_eq!(b.remaining_daily(), 10.0);
        assert_eq!(b.remaining_monthly(), 95.0);
    }

    #[test]
    fn frozen_budget_rejects_spend() {
        let mut b = Budget::new(10.0, 100.0);
        b.freeze();
        let err = b.spend(1.0, DeviceTier::Cloud, "x".into()).unwrap_err();
        assert!(matches!(err, BudgetError::Frozen));
        assert!(!b.can_afford(1.0));
    }

    #[test]
    fn freeze_unfreeze() {
        let mut b = Budget::new(10.0, 100.0);
        b.freeze();
        assert!(b.is_frozen());
        b.unfreeze();
        assert!(!b.is_frozen());
        assert!(b.can_afford(1.0));
    }

    #[test]
    fn daily_report() {
        let mut b = Budget::new(10.0, 100.0);
        b.spend(3.0, DeviceTier::Cortex, "a".into()).unwrap();
        b.spend(2.0, DeviceTier::Cloud, "b".into()).unwrap();
        let report = b.daily_report();
        assert_eq!(report.spent_today, 5.0);
        assert_eq!(report.daily_remaining, 5.0);
        assert_eq!(report.transaction_count, 2);
    }

    #[test]
    fn transactions_recorded() {
        let mut b = Budget::new(10.0, 100.0);
        b.spend(1.0, DeviceTier::Cortex, "reason".into()).unwrap();
        let txn = &b.transactions()[0];
        assert_eq!(txn.amount, 1.0);
        assert_eq!(txn.tier, DeviceTier::Cortex);
        assert_eq!(txn.reason, "reason");
    }
}
