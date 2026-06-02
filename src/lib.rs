#![deny(unsafe_code)]

pub mod budget;
pub mod escalation;
pub mod jeff;
pub mod rebalance;

pub use budget::{Budget, BudgetError, BudgetState, Transaction};
pub use escalation::{EscalationChain, EscalationError, EscalationEvent, EscalationTier};
pub use jeff::{DivergenceEvent, JeffPredictor, Prediction};
pub use rebalance::{DeviceState, RebalanceDecision, Rebalancer, UserPreferences};
