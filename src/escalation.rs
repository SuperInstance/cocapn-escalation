use std::collections::HashMap;
use std::time::{Duration, Instant};

use cocapn_core::{Capability, DeviceTier};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EscalationError {
    #[error("already at maximum tier")]
    MaxTier,
    #[error("already at minimum tier")]
    MinTier,
    #[error("cooldown period has not elapsed")]
    CooldownActive,
    #[error("no tiers configured")]
    NoTiers,
}

#[derive(Debug, Clone)]
pub struct EscalationTier {
    pub tier: DeviceTier,
    pub device_id: String,
    pub capabilities: Vec<Capability>,
    pub cost_per_invocation: Option<f64>,
    pub estimated_latency: Duration,
}

#[derive(Debug, Clone)]
pub enum EscalationEvent {
    Escalated {
        from: DeviceTier,
        to: DeviceTier,
        reason: String,
    },
    DeEscalated {
        from: DeviceTier,
        to: DeviceTier,
        reason: String,
    },
    MaxTier {
        tier: DeviceTier,
        unresolved: String,
    },
    CostAlert {
        daily_spend: f64,
        budget: f64,
    },
}

pub struct EscalationChain {
    tiers: Vec<EscalationTier>,
    current_tier: usize,
    cooldown: Duration,
    last_escalation: Option<Instant>,
    escalation_count: HashMap<String, u32>,
    events: Vec<EscalationEvent>,
}

impl EscalationChain {
    pub fn new(tiers: Vec<EscalationTier>) -> Self {
        Self {
            escalation_count: tiers.iter().map(|t| (t.device_id.clone(), 0)).collect(),
            tiers,
            current_tier: 0,
            cooldown: Duration::from_secs(30),
            last_escalation: None,
            events: Vec::new(),
        }
    }

    pub fn with_cooldown(mut self, cooldown: Duration) -> Self {
        self.cooldown = cooldown;
        self
    }

    pub fn trigger(&mut self, reason: String) -> Result<&EscalationTier, EscalationError> {
        if self.tiers.is_empty() {
            return Err(EscalationError::NoTiers);
        }

        if self.current_tier >= self.tiers.len() - 1 {
            let event = EscalationEvent::MaxTier {
                tier: self.tiers[self.current_tier].tier,
                unresolved: reason,
            };
            self.events.push(event);
            return Err(EscalationError::MaxTier);
        }

        if let Some(last) = self.last_escalation {
            if last.elapsed() < self.cooldown {
                return Err(EscalationError::CooldownActive);
            }
        }

        let from_tier = self.tiers[self.current_tier].tier;
        self.current_tier += 1;
        let to_tier = self.tiers[self.current_tier].tier;
        self.last_escalation = Some(Instant::now());

        let device_id = self.tiers[self.current_tier].device_id.clone();
        *self.escalation_count.entry(device_id).or_insert(0) += 1;

        let event = EscalationEvent::Escalated {
            from: from_tier,
            to: to_tier,
            reason,
        };
        self.events.push(event);

        log::info!("Escalated to tier {:?}", self.tiers[self.current_tier].tier);
        Ok(&self.tiers[self.current_tier])
    }

    pub fn resolve(&mut self) -> Result<&EscalationTier, EscalationError> {
        if self.tiers.is_empty() {
            return Err(EscalationError::NoTiers);
        }

        if self.current_tier == 0 {
            return Err(EscalationError::MinTier);
        }

        if let Some(last) = self.last_escalation {
            if last.elapsed() < self.cooldown {
                return Err(EscalationError::CooldownActive);
            }
        }

        let from_tier = self.tiers[self.current_tier].tier;
        self.current_tier -= 1;
        let to_tier = self.tiers[self.current_tier].tier;
        self.last_escalation = Some(Instant::now());

        let event = EscalationEvent::DeEscalated {
            from: from_tier,
            to: to_tier,
            reason: "resolved".to_string(),
        };
        self.events.push(event);

        Ok(&self.tiers[self.current_tier])
    }

    pub fn can_escalate(&self) -> bool {
        if self.tiers.is_empty() {
            return false;
        }
        if self.current_tier >= self.tiers.len() - 1 {
            return false;
        }
        if let Some(last) = self.last_escalation {
            if last.elapsed() < self.cooldown {
                return false;
            }
        }
        true
    }

    pub fn current(&self) -> &EscalationTier {
        &self.tiers[self.current_tier]
    }

    pub fn events(&self) -> &[EscalationEvent] {
        &self.events
    }

    pub fn escalation_count(&self, device_id: &str) -> u32 {
        *self.escalation_count.get(device_id).unwrap_or(&0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tiers() -> Vec<EscalationTier> {
        vec![
            EscalationTier {
                tier: DeviceTier::Reflex,
                device_id: "esp-01".into(),
                capabilities: vec![Capability::Sense],
                cost_per_invocation: None,
                estimated_latency: Duration::from_millis(5),
            },
            EscalationTier {
                tier: DeviceTier::Backbone,
                device_id: "pi-01".into(),
                capabilities: vec![Capability::Sense, Capability::Predict],
                cost_per_invocation: None,
                estimated_latency: Duration::from_millis(50),
            },
            EscalationTier {
                tier: DeviceTier::Cortex,
                device_id: "jetson-01".into(),
                capabilities: vec![Capability::Predict, Capability::Train],
                cost_per_invocation: Some(0.001),
                estimated_latency: Duration::from_millis(200),
            },
            EscalationTier {
                tier: DeviceTier::Cloud,
                device_id: "cloud-01".into(),
                capabilities: vec![Capability::Train, Capability::Communicate],
                cost_per_invocation: Some(0.05),
                estimated_latency: Duration::from_secs(2),
            },
        ]
    }

    #[test]
    fn new_chain_starts_at_first_tier() {
        let chain = EscalationChain::new(test_tiers());
        assert_eq!(chain.current().device_id, "esp-01");
        assert_eq!(chain.current().tier, DeviceTier::Reflex);
    }

    #[test]
    fn trigger_escalates_one_tier() {
        let mut chain = EscalationChain::new(test_tiers()).with_cooldown(Duration::ZERO);
        let tier = chain.trigger("sensor anomaly".into()).unwrap();
        assert_eq!(tier.tier, DeviceTier::Backbone);
        assert_eq!(chain.current().device_id, "pi-01");
    }

    #[test]
    fn trigger_multiple_escalations() {
        let mut chain = EscalationChain::new(test_tiers()).with_cooldown(Duration::ZERO);
        chain.trigger("first".into()).unwrap();
        chain.trigger("second".into()).unwrap();
        assert_eq!(chain.current().tier, DeviceTier::Cortex);
    }

    #[test]
    fn trigger_max_tier_returns_error() {
        let mut chain = EscalationChain::new(test_tiers()).with_cooldown(Duration::ZERO);
        chain.trigger("1".into()).unwrap();
        chain.trigger("2".into()).unwrap();
        chain.trigger("3".into()).unwrap();
        let err = chain.trigger("4".into()).unwrap_err();
        assert!(matches!(err, EscalationError::MaxTier));
    }

    #[test]
    fn max_tier_generates_event() {
        let mut chain = EscalationChain::new(test_tiers()).with_cooldown(Duration::ZERO);
        chain.trigger("1".into()).unwrap();
        chain.trigger("2".into()).unwrap();
        chain.trigger("3".into()).unwrap();
        let _ = chain.trigger("4".into());
        assert!(matches!(
            chain.events().last(),
            Some(EscalationEvent::MaxTier { .. })
        ));
    }

    #[test]
    fn cooldown_blocks_escalation() {
        let mut chain = EscalationChain::new(test_tiers()).with_cooldown(Duration::from_secs(600));
        chain.trigger("first".into()).unwrap();
        let err = chain.trigger("second".into()).unwrap_err();
        assert!(matches!(err, EscalationError::CooldownActive));
    }

    #[test]
    fn cooldown_blocks_deescalation() {
        let mut chain = EscalationChain::new(test_tiers()).with_cooldown(Duration::from_secs(600));
        chain.trigger("first".into()).unwrap();
        let err = chain.resolve().unwrap_err();
        assert!(matches!(err, EscalationError::CooldownActive));
    }

    #[test]
    fn resolve_deescalates() {
        let mut chain = EscalationChain::new(test_tiers()).with_cooldown(Duration::ZERO);
        chain.trigger("up".into()).unwrap();
        chain.resolve().unwrap();
        assert_eq!(chain.current().tier, DeviceTier::Reflex);
    }

    #[test]
    fn resolve_at_bottom_errors() {
        let mut chain = EscalationChain::new(test_tiers());
        let err = chain.resolve().unwrap_err();
        assert!(matches!(err, EscalationError::MinTier));
    }

    #[test]
    fn can_escalate_at_bottom() {
        let chain = EscalationChain::new(test_tiers()).with_cooldown(Duration::ZERO);
        assert!(chain.can_escalate());
    }

    #[test]
    fn can_escalate_false_at_top() {
        let mut chain = EscalationChain::new(test_tiers()).with_cooldown(Duration::ZERO);
        chain.trigger("1".into()).unwrap();
        chain.trigger("2".into()).unwrap();
        chain.trigger("3".into()).unwrap();
        assert!(!chain.can_escalate());
    }

    #[test]
    fn empty_tiers_errors() {
        let mut chain = EscalationChain::new(vec![]);
        assert!(matches!(
            chain.trigger("x".into()).unwrap_err(),
            EscalationError::NoTiers
        ));
        assert!(matches!(
            chain.resolve().unwrap_err(),
            EscalationError::NoTiers
        ));
        assert!(!chain.can_escalate());
    }

    #[test]
    fn escalation_count_tracks() {
        let mut chain = EscalationChain::new(test_tiers()).with_cooldown(Duration::ZERO);
        chain.trigger("1".into()).unwrap();
        chain.trigger("2".into()).unwrap();
        assert_eq!(chain.escalation_count("pi-01"), 1);
        assert_eq!(chain.escalation_count("jetson-01"), 1);
        assert_eq!(chain.escalation_count("esp-01"), 0);
    }

    #[test]
    fn escalated_event_has_reason() {
        let mut chain = EscalationChain::new(test_tiers()).with_cooldown(Duration::ZERO);
        chain.trigger("anomaly detected".into()).unwrap();
        if let Some(EscalationEvent::Escalated { reason, .. }) = chain.events().first() {
            assert_eq!(reason, "anomaly detected");
        } else {
            panic!("expected Escalated event");
        }
    }
}
