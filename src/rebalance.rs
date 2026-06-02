use std::collections::HashMap;

use cocapn_core::{Capability, DeviceTier};

#[derive(Debug, Clone)]
pub struct DeviceState {
    pub id: String,
    pub tier: DeviceTier,
    pub online: bool,
    pub current_load: f64,
    pub latency: std::time::Duration,
    pub cost_per_unit: f64,
    pub capabilities: Vec<Capability>,
}

#[derive(Debug, Clone)]
pub struct UserPreferences {
    pub prefer_local: bool,
    pub max_cloud_spend: f64,
    pub max_latency: std::time::Duration,
    pub allow_degraded: bool,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            prefer_local: true,
            max_cloud_spend: 50.0,
            max_latency: std::time::Duration::from_secs(5),
            allow_degraded: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TaskRequirements {
    pub required_capability: Capability,
    pub max_latency: Option<std::time::Duration>,
    pub max_cost: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct RebalanceDecision {
    pub use_device: String,
    pub reason: String,
    pub estimated_cost: f64,
    pub estimated_latency: std::time::Duration,
}

pub struct Rebalancer {
    devices: HashMap<String, DeviceState>,
    preferences: UserPreferences,
}

impl Rebalancer {
    pub fn new(preferences: UserPreferences) -> Self {
        Self {
            devices: HashMap::new(),
            preferences,
        }
    }

    pub fn add_device(&mut self, device: DeviceState) {
        self.devices.insert(device.id.clone(), device);
    }

    pub fn remove_device(&mut self, id: &str) {
        self.devices.remove(id);
    }

    pub fn evaluate(&self, req: &TaskRequirements) -> Option<RebalanceDecision> {
        let mut candidates: Vec<_> = self
            .devices
            .values()
            .filter(|d| {
                d.online
                    && d.capabilities.contains(&req.required_capability)
                    && d.current_load < 1.0
            })
            .collect();

        if candidates.is_empty() {
            return None;
        }

        if let Some(max_lat) = req.max_latency {
            candidates.retain(|d| d.latency <= max_lat);
        }

        if let Some(max_cost) = req.max_cost {
            candidates.retain(|d| d.cost_per_unit <= max_cost);
        }

        candidates.retain(|d| d.latency <= self.preferences.max_latency);

        if candidates.is_empty() {
            return None;
        }

        candidates.sort_by(|a, b| {
            if self.preferences.prefer_local {
                a.tier.cmp(&b.tier)
            } else {
                a.cost_per_unit
                    .partial_cmp(&b.cost_per_unit)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }
        });

        let best = candidates.into_iter().next().unwrap();

        Some(RebalanceDecision {
            use_device: best.id.clone(),
            reason: format!(
                "selected {:?} device (load: {:.0}%)",
                best.tier,
                best.current_load * 100.0
            ),
            estimated_cost: best.cost_per_unit,
            estimated_latency: best.latency,
        })
    }

    pub fn device_went_offline(&mut self, id: &str) -> Vec<RebalanceDecision> {
        if let Some(device) = self.devices.get_mut(id) {
            device.online = false;
        }

        let offline_device = match self.devices.get(id) {
            Some(d) => d.clone(),
            None => return vec![],
        };

        let mut decisions = Vec::new();
        for cap in &offline_device.capabilities {
            let req = TaskRequirements {
                required_capability: *cap,
                max_latency: None,
                max_cost: None,
            };
            if let Some(decision) = self.evaluate(&req) {
                decisions.push(decision);
            }
        }
        decisions
    }

    pub fn device_came_online(&mut self, id: &str) -> Vec<RebalanceDecision> {
        if let Some(device) = self.devices.get_mut(id) {
            device.online = true;
        }

        if !self.preferences.prefer_local {
            return vec![];
        }

        let target = match self.devices.get(id) {
            Some(d) => d.clone(),
            None => return vec![],
        };

        let mut decisions = Vec::new();
        for cap in &target.capabilities {
            for other in self.devices.values() {
                if other.online
                    && other.id != target.id
                    && other.tier > target.tier
                    && other.capabilities.contains(cap)
                {
                    decisions.push(RebalanceDecision {
                        use_device: target.id.clone(),
                        reason: format!(
                            "migrating {:?} tasks from {:?} to {:?} (prefer local)",
                            cap, other.tier, target.tier
                        ),
                        estimated_cost: target.cost_per_unit,
                        estimated_latency: target.latency,
                    });
                    break;
                }
            }
        }
        decisions
    }

    pub fn update_load(&mut self, id: &str, new_load: f64) {
        if let Some(device) = self.devices.get_mut(id) {
            device.current_load = new_load.clamp(0.0, 1.0);
        }
    }

    pub fn get_device(&self, id: &str) -> Option<&DeviceState> {
        self.devices.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn pi_device() -> DeviceState {
        DeviceState {
            id: "pi-01".into(),
            tier: DeviceTier::Backbone,
            online: true,
            current_load: 0.3,
            latency: Duration::from_millis(50),
            cost_per_unit: 0.0,
            capabilities: vec![Capability::Sense, Capability::Predict],
        }
    }

    fn jetson_device() -> DeviceState {
        DeviceState {
            id: "jetson-01".into(),
            tier: DeviceTier::Cortex,
            online: true,
            current_load: 0.5,
            latency: Duration::from_millis(200),
            cost_per_unit: 0.001,
            capabilities: vec![Capability::Predict, Capability::Train],
        }
    }

    fn cloud_device() -> DeviceState {
        DeviceState {
            id: "cloud-01".into(),
            tier: DeviceTier::Cloud,
            online: true,
            current_load: 0.1,
            latency: Duration::from_secs(2),
            cost_per_unit: 0.05,
            capabilities: vec![Capability::Train, Capability::Communicate],
        }
    }

    fn make_rebalancer() -> Rebalancer {
        let mut r = Rebalancer::new(UserPreferences {
            prefer_local: true,
            max_cloud_spend: 50.0,
            max_latency: Duration::from_secs(5),
            allow_degraded: false,
        });
        r.add_device(pi_device());
        r.add_device(jetson_device());
        r.add_device(cloud_device());
        r
    }

    #[test]
    fn evaluate_picks_local_first() {
        let r = make_rebalancer();
        let req = TaskRequirements {
            required_capability: Capability::Predict,
            max_latency: None,
            max_cost: None,
        };
        let dec = r.evaluate(&req).unwrap();
        assert_eq!(dec.use_device, "pi-01");
    }

    #[test]
    fn evaluate_picks_capability_match() {
        let r = make_rebalancer();
        let req = TaskRequirements {
            required_capability: Capability::Train,
            max_latency: None,
            max_cost: None,
        };
        let dec = r.evaluate(&req).unwrap();
        assert_eq!(dec.use_device, "jetson-01");
    }

    #[test]
    fn evaluate_no_match_returns_none() {
        let r = make_rebalancer();
        let req = TaskRequirements {
            required_capability: Capability::Train,
            max_latency: Some(Duration::from_millis(10)),
            max_cost: None,
        };
        assert!(r.evaluate(&req).is_none());
    }

    #[test]
    fn device_offline_triggers_rebalance() {
        let mut r = make_rebalancer();
        let decisions = r.device_went_offline("pi-01");
        assert!(!decisions.is_empty());
        assert!(!r.get_device("pi-01").unwrap().online);
    }

    #[test]
    fn device_online_migrates_back() {
        let mut r = make_rebalancer();
        r.device_went_offline("pi-01");
        let decisions = r.device_came_online("pi-01");
        assert!(decisions.iter().any(|d| d.use_device == "pi-01"));
    }

    #[test]
    fn update_load() {
        let mut r = make_rebalancer();
        r.update_load("pi-01", 0.95);
        assert!((r.get_device("pi-01").unwrap().current_load - 0.95).abs() < 0.001);
    }

    #[test]
    fn full_load_device_skipped() {
        let mut r = make_rebalancer();
        r.update_load("pi-01", 1.0);
        let req = TaskRequirements {
            required_capability: Capability::Predict,
            max_latency: None,
            max_cost: None,
        };
        let dec = r.evaluate(&req).unwrap();
        assert_eq!(dec.use_device, "jetson-01");
    }

    #[test]
    fn prefer_local_false_sorts_by_cost() {
        let mut r = Rebalancer::new(UserPreferences {
            prefer_local: false,
            max_cloud_spend: 50.0,
            max_latency: Duration::from_secs(5),
            allow_degraded: false,
        });
        r.add_device(pi_device());
        r.add_device(jetson_device());
        let req = TaskRequirements {
            required_capability: Capability::Predict,
            max_latency: None,
            max_cost: None,
        };
        let dec = r.evaluate(&req).unwrap();
        assert_eq!(dec.use_device, "pi-01");
    }

    #[test]
    fn cost_filter_applied() {
        let r = make_rebalancer();
        let req = TaskRequirements {
            required_capability: Capability::Train,
            max_latency: None,
            max_cost: Some(0.01),
        };
        // jetson costs 0.001 (ok), cloud costs 0.05 (too much)
        // jetson should still be selected
        let dec = r.evaluate(&req).unwrap();
        assert_eq!(dec.use_device, "jetson-01");
    }

    #[test]
    fn all_devices_offline_returns_none() {
        let mut r = make_rebalancer();
        r.device_went_offline("pi-01");
        r.device_went_offline("jetson-01");
        r.device_went_offline("cloud-01");
        let req = TaskRequirements {
            required_capability: Capability::Predict,
            max_latency: None,
            max_cost: None,
        };
        assert!(r.evaluate(&req).is_none());
    }
}
