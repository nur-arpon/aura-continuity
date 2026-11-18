//! The real decider: the stage-1 race orchestrator (phone + watch, hands-free policy), used as a
//! library. Nothing about the race or the policy is re-implemented here.

use crate::engine::{Decider, Decision};
use crate::proto::reason;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use unlock_pc::orchestrator::{Budget, Orchestrator, Transport};
use unlock_pc::transports::{BleRssiSampler, BleTransport, RfcommTransport, UsbTetherTransport, WatchTransport};
use unlock_pc::{pairing, policy, protocol};

/// Race budget used by the helper. The pipe answer is due 1.8 s after connect, so the warm race
/// decides by 1.7 s; a cold race may run longer and its approval is kept for the next request
/// (engine grant). Everything else is stage-1's default.
pub fn helper_budget() -> Budget {
    Budget { race: Duration::from_millis(1700), ..Budget::default() }
}

pub struct PhoneDecider {
    orch: Mutex<Option<Orchestrator>>,
    log: Arc<dyn Fn(&str) + Send + Sync>,
}

impl PhoneDecider {
    pub fn new(log: Arc<dyn Fn(&str) + Send + Sync>) -> Self {
        Self { orch: Mutex::new(None), log }
    }

    fn build() -> Result<Orchestrator, String> {
        let secret = pairing::store::load().map_err(|e| format!("not paired: {e}"))?;
        let keys = protocol::derive(&secret);
        let ts: Vec<Arc<dyn Transport>> = vec![
            Arc::new(RfcommTransport::new()),
            Arc::new(WatchTransport::new()),
            Arc::new(BleTransport::new(false)),
            Arc::new(UsbTetherTransport),
        ];
        let mut o = Orchestrator::new(ts, protocol::Verifier::new(keys, Duration::from_secs(30)), helper_budget(), policy::Policy::default());
        o.watch_key = WatchTransport::load_key().map(Arc::new);
        o.sampler = Some(Arc::new(BleRssiSampler::default()));
        Ok(o)
    }

    fn with<R>(&self, f: impl FnOnce(&mut Orchestrator) -> R) -> Result<R, String> {
        let mut g = self.orch.lock().unwrap_or_else(|p| p.into_inner());
        if g.is_none() {
            *g = Some(Self::build()?);
        }
        Ok(f(g.as_mut().unwrap()))
    }
}

/// Map a stage-1 report to one pipe reason.
pub fn reason_of(winner: bool, outcomes: &[Result<u8, String>]) -> Decision {
    if winner {
        return Decision::Approved;
    }
    let errs: Vec<&str> = outcomes.iter().filter_map(|o| o.as_ref().err().map(|s| s.as_str())).collect();
    if errs.iter().any(|e| e.contains("denied on phone")) {
        Decision::NotApproved(reason::DENIED)
    } else if errs.iter().any(|e| e.contains("hands-free not possible") || e.contains("not trusted") || e.contains("not ready") || e.contains("user did not approve")) {
        Decision::NotApproved(reason::NOT_NOW)
    } else {
        Decision::NotApproved(reason::NO_DEVICE)
    }
}

impl Decider for PhoneDecider {
    fn decide(&self) -> Decision {
        if !crate::HANDS_FREE.load(std::sync::atomic::Ordering::SeqCst) {
            (self.log)("decide: hands-free sign-in is switched off in Aura -> NOT_NOW (use the PIN)");
            return Decision::NotApproved(reason::NOT_NOW);
        }
        match self.with(|o| o.attempt()) {
            Err(e) => {
                (self.log)(&format!("decide: {e}"));
                if e.starts_with("not paired") {
                    Decision::NotApproved(reason::NOT_PAIRED)
                } else {
                    Decision::NotApproved(reason::INTERNAL)
                }
            }
            Ok(r) => {
                let steps: Vec<String> = r
                    .steps
                    .iter()
                    .map(|s| match &s.outcome {
                        Ok(l) => format!("{}=OK({} at {}ms)", s.transport, protocol::level_name(*l), s.elapsed.as_millis()),
                        Err(e) => format!("{}=FAIL({e} at {}ms)", s.transport, s.elapsed.as_millis()),
                    })
                    .collect();
                (self.log)(&format!("race winner={} total={}ms health=\"{}\" | {}", r.winner.unwrap_or("-"), r.total.as_millis(), r.health.message(), steps.join(" ; ")));
                let outcomes: Vec<Result<u8, String>> = r.steps.into_iter().map(|s| s.outcome).collect();
                reason_of(r.winner.is_some(), &outcomes)
            }
        }
    }

    fn prewarm(&self) {
        if !crate::HANDS_FREE.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        // skip if a decision holds the orchestrator: its race is warming the links anyway
        let Ok(mut g) = self.orch.try_lock() else { return };
        if g.is_none() {
            match Self::build() {
                Ok(o) => *g = Some(o),
                Err(e) => return (self.log)(&format!("prewarm: {e}")),
            }
        }
        let legs = g.as_ref().unwrap().prewarm();
        (self.log)(&format!("prewarm started: {legs:?}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_fits_the_pipe_deadline() {
        let b = helper_budget();
        assert!(b.race < crate::engine::Config::default().deadline);
    }

    #[test]
    fn hands_free_off_answers_not_now_without_touching_a_radio() {
        use std::sync::atomic::Ordering::SeqCst;
        let d = PhoneDecider::new(Arc::new(|_| {}));
        crate::HANDS_FREE.store(false, SeqCst);
        let r = d.decide();
        d.prewarm();
        crate::HANDS_FREE.store(true, SeqCst);
        assert_eq!(r, Decision::NotApproved(reason::NOT_NOW));
        assert!(d.orch.lock().unwrap().is_none(), "nothing was built: no pairing read, no radio");
    }

    #[test]
    fn reasons() {
        assert_eq!(reason_of(true, &[]), Decision::Approved);
        assert_eq!(reason_of(false, &[Err("denied on phone".into())]), Decision::NotApproved(reason::DENIED));
        assert_eq!(reason_of(false, &[Err("device says hands-free not possible now".into()), Err("x".into())]), Decision::NotApproved(reason::NOT_NOW));
        assert_eq!(reason_of(false, &[Err("not available: radio off".into())]), Decision::NotApproved(reason::NO_DEVICE));
        assert_eq!(reason_of(false, &[]), Decision::NotApproved(reason::NO_DEVICE));
    }
}
