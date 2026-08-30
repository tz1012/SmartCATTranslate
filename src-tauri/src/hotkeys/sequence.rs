use std::time::Duration;

use uuid::Uuid;

use super::{Chord, HotkeyBinding, Trigger};

pub const MAX_SEQUENCE_BINDINGS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyDevice(pub u64);

impl KeyDevice {
    const UNSPECIFIED: Self = Self(0);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEventPhase {
    Down,
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub chord: Chord,
    pub phase: KeyEventPhase,
    pub repeat: bool,
    pub device: KeyDevice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPropagation {
    PassThrough,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceOutcome {
    pub binding_ids: Vec<Uuid>,
    pub propagation: KeyPropagation,
}

impl SequenceOutcome {
    fn pass_through(binding_ids: Vec<Uuid>) -> Self {
        Self {
            binding_ids,
            propagation: KeyPropagation::PassThrough,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SequenceEngineError {
    #[error("too many sequence bindings")]
    TooManyBindings,
    #[error("key event timestamp moved backwards")]
    NonMonotonicTimestamp,
}

#[derive(Debug)]
struct Cursor {
    id: Uuid,
    steps: Vec<Chord>,
    timeout: Duration,
    next_step: usize,
    last_at: Option<Duration>,
    device: Option<KeyDevice>,
}

#[derive(Clone, Debug)]
struct CompletedMatch {
    id: Uuid,
    steps: Vec<Chord>,
    at: Duration,
    device: KeyDevice,
}

#[derive(Clone, Debug)]
struct PendingMatch {
    id: Uuid,
    steps: Vec<Chord>,
    completed_at: Duration,
    device: KeyDevice,
    timeout: Duration,
}

impl Cursor {
    fn new(binding: HotkeyBinding) -> Option<Self> {
        let Trigger::Sequence { steps, timeout_ms } = binding.trigger else {
            return None;
        };
        Some(Self {
            id: binding.id,
            steps,
            timeout: Duration::from_millis(timeout_ms),
            next_step: 0,
            last_at: None,
            device: None,
        })
    }

    fn reset(&mut self) {
        self.next_step = 0;
        self.last_at = None;
        self.device = None;
    }

    fn start_if_first(&mut self, chord: Chord, at: Duration, device: KeyDevice) {
        if self.steps.first() == Some(&chord) {
            self.next_step = 1;
            self.last_at = Some(at);
            self.device = Some(device);
        }
    }

    fn advance(&mut self, chord: Chord, at: Duration, device: KeyDevice) -> Option<Uuid> {
        if self.next_step == 0 {
            self.start_if_first(chord, at, device);
            return None;
        }

        let expired = self
            .last_at
            .is_some_and(|last_at| at.saturating_sub(last_at) > self.timeout);
        if expired || self.device != Some(device) {
            self.reset();
            self.start_if_first(chord, at, device);
            return None;
        }

        if self.steps.get(self.next_step) != Some(&chord) {
            self.reset();
            self.start_if_first(chord, at, device);
            return None;
        }

        self.next_step += 1;
        self.last_at = Some(at);
        if self.next_step != self.steps.len() {
            return None;
        }

        let id = self.id;
        self.reset();
        self.start_if_first(chord, at, device);
        Some(id)
    }

    fn expire(&mut self, at: Duration) {
        if self
            .last_at
            .is_some_and(|last_at| at.saturating_sub(last_at) > self.timeout)
        {
            self.reset();
        }
    }

    fn actively_extends(&self, steps: &[Chord], device: KeyDevice) -> bool {
        self.steps.len() > steps.len()
            && self.steps.starts_with(steps)
            && self.next_step == steps.len()
            && self.device == Some(device)
    }
}

#[derive(Debug)]
pub struct SequenceEngine {
    cursors: Vec<Cursor>,
    pending: Vec<PendingMatch>,
    last_event_at: Option<Duration>,
    suspended: bool,
}

impl SequenceEngine {
    pub fn new(bindings: Vec<HotkeyBinding>) -> Result<Self, SequenceEngineError> {
        let cursors: Vec<_> = bindings.into_iter().filter_map(Cursor::new).collect();
        if cursors.len() > MAX_SEQUENCE_BINDINGS {
            return Err(SequenceEngineError::TooManyBindings);
        }
        Ok(Self {
            cursors,
            pending: Vec::new(),
            last_event_at: None,
            suspended: false,
        })
    }

    pub fn on_key(&mut self, chord: Chord, at: Duration) -> Vec<Uuid> {
        self.on_event(
            KeyEvent {
                chord,
                phase: KeyEventPhase::Down,
                repeat: false,
                device: KeyDevice::UNSPECIFIED,
            },
            at,
        )
        .map_or_else(|_| Vec::new(), |outcome| outcome.binding_ids)
    }

    pub fn on_event(
        &mut self,
        event: KeyEvent,
        at: Duration,
    ) -> Result<SequenceOutcome, SequenceEngineError> {
        if self.last_event_at.is_some_and(|last_at| at < last_at) {
            self.clear_cursors();
            return Err(SequenceEngineError::NonMonotonicTimestamp);
        }
        self.last_event_at = Some(at);

        if self.suspended || event.phase != KeyEventPhase::Down || event.repeat {
            return Ok(SequenceOutcome::pass_through(Vec::new()));
        }

        let mut binding_ids = self.take_expired_pending(at);
        for cursor in &mut self.cursors {
            cursor.expire(at);
        }

        let mut completed = Vec::new();
        for cursor in &mut self.cursors {
            if let Some(id) = cursor.advance(event.chord, at, event.device) {
                completed.push(CompletedMatch {
                    id,
                    steps: cursor.steps.clone(),
                    at,
                    device: event.device,
                });
            }
        }

        let previous_pending = std::mem::take(&mut self.pending);
        for pending in previous_pending {
            let extended_by_completion = completed.iter().any(|matched| {
                matched.steps.len() > pending.steps.len()
                    && matched.steps.starts_with(&pending.steps)
                    && matched.device == pending.device
            });
            if extended_by_completion {
                continue;
            }
            if self.has_active_extension(&pending.steps, pending.device) {
                self.pending.push(pending);
            } else {
                binding_ids.push(pending.id);
            }
        }

        for matched in completed.iter().cloned() {
            let shadowed_by_longer = completed.iter().any(|other| {
                other.steps.len() > matched.steps.len()
                    && other.steps.starts_with(&matched.steps)
                    && other.device == matched.device
            });
            if shadowed_by_longer {
                continue;
            }
            if self.has_active_extension(&matched.steps, matched.device) {
                let timeout = self
                    .cursors
                    .iter()
                    .filter(|cursor| {
                        cursor.steps.len() > matched.steps.len()
                            && cursor.steps.starts_with(&matched.steps)
                    })
                    .map(|cursor| cursor.timeout)
                    .max()
                    .unwrap_or_default();
                self.pending.push(PendingMatch {
                    id: matched.id,
                    steps: matched.steps,
                    completed_at: matched.at,
                    device: matched.device,
                    timeout,
                });
            } else {
                binding_ids.push(matched.id);
            }
        }
        Ok(SequenceOutcome::pass_through(binding_ids))
    }

    pub fn on_time(&mut self, at: Duration) -> Result<Vec<Uuid>, SequenceEngineError> {
        if self.last_event_at.is_some_and(|last_at| at < last_at) {
            self.clear_cursors();
            return Err(SequenceEngineError::NonMonotonicTimestamp);
        }
        self.last_event_at = Some(at);
        for cursor in &mut self.cursors {
            cursor.expire(at);
        }
        Ok(self.take_expired_pending(at))
    }

    pub fn set_suspended(&mut self, suspended: bool) {
        self.suspended = suspended;
        self.clear_cursors();
    }

    pub fn on_focus_lost(&mut self) {
        self.clear_cursors();
    }

    pub fn reset(&mut self) {
        self.clear_cursors();
        self.last_event_at = None;
    }

    fn clear_cursors(&mut self) {
        for cursor in &mut self.cursors {
            cursor.reset();
        }
        self.pending.clear();
    }

    fn has_active_extension(&self, steps: &[Chord], device: KeyDevice) -> bool {
        self.cursors
            .iter()
            .any(|cursor| cursor.actively_extends(steps, device))
    }

    fn take_expired_pending(&mut self, at: Duration) -> Vec<Uuid> {
        let mut expired = Vec::new();
        self.pending.retain(|pending| {
            if at.saturating_sub(pending.completed_at) > pending.timeout {
                expired.push(pending.id);
                false
            } else {
                true
            }
        });
        expired
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use uuid::Uuid;

    use super::{
        KeyDevice, KeyEvent, KeyEventPhase, KeyPropagation, SequenceEngine, SequenceEngineError,
        MAX_SEQUENCE_BINDINGS,
    };
    use crate::hotkeys::{parse_trigger, HotkeyAction, HotkeyBinding, Modifiers, Trigger};

    fn binding(id: Uuid, trigger: &str) -> HotkeyBinding {
        HotkeyBinding {
            id,
            trigger: parse_trigger(trigger).unwrap(),
            action: HotkeyAction::TranslateSelection,
            profile_id: Uuid::nil(),
            force: false,
        }
    }

    fn chord(trigger: &str) -> crate::hotkeys::Chord {
        let (parsed, remove_test_modifier) = if trigger.contains('+') {
            (parse_trigger(trigger).unwrap(), false)
        } else {
            (parse_trigger(&format!("Ctrl+{trigger}")).unwrap(), true)
        };
        match parsed {
            Trigger::Chord { mut chord } => {
                if remove_test_modifier {
                    chord.modifiers = Modifiers::default();
                }
                chord
            }
            Trigger::Sequence { .. } => panic!("expected one chord"),
        }
    }

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn event(trigger: &str, phase: KeyEventPhase, repeat: bool, device: u64) -> KeyEvent {
        KeyEvent {
            chord: chord(trigger),
            phase,
            repeat,
            device: KeyDevice(device),
        }
    }

    #[test]
    fn fires_ctrl_c_then_c_once_inside_the_inclusive_timeout() {
        let id = Uuid::from_u128(1);
        let mut engine = SequenceEngine::new(vec![binding(id, "Ctrl+C, C")]).unwrap();

        assert!(engine.on_key(chord("Ctrl+C"), ms(0)).is_empty());
        assert_eq!(engine.on_key(chord("C"), ms(650)), vec![id]);
        assert!(engine.on_key(chord("C"), ms(651)).is_empty());
    }

    #[test]
    fn timeout_and_wrong_keys_reset_then_re_evaluate_the_current_key() {
        let id = Uuid::from_u128(2);
        let mut engine = SequenceEngine::new(vec![binding(id, "Ctrl+C, C, V")]).unwrap();

        engine.on_key(chord("Ctrl+C"), ms(0));
        assert!(engine.on_key(chord("C"), ms(651)).is_empty());
        assert!(engine.on_key(chord("V"), ms(700)).is_empty());

        // The mismatch is also the first chord of a fresh attempt.
        engine.on_key(chord("Ctrl+C"), ms(800));
        engine.on_key(chord("Ctrl+C"), ms(900));
        engine.on_key(chord("C"), ms(1_000));
        assert_eq!(engine.on_key(chord("V"), ms(1_100)), vec![id]);
    }

    #[test]
    fn observes_key_phases_without_consuming_host_events_and_ignores_repeats() {
        let id = Uuid::from_u128(3);
        let mut engine = SequenceEngine::new(vec![binding(id, "Ctrl+C, C")]).unwrap();

        let first = engine
            .on_event(event("Ctrl+C", KeyEventPhase::Down, false, 1), ms(0))
            .unwrap();
        assert_eq!(first.propagation, KeyPropagation::PassThrough);
        assert!(first.binding_ids.is_empty());

        let released = engine
            .on_event(event("Ctrl+C", KeyEventPhase::Up, false, 1), ms(10))
            .unwrap();
        let repeated = engine
            .on_event(event("C", KeyEventPhase::Down, true, 1), ms(20))
            .unwrap();
        assert_eq!(released.propagation, KeyPropagation::PassThrough);
        assert_eq!(repeated.propagation, KeyPropagation::PassThrough);
        assert!(released.binding_ids.is_empty());
        assert!(repeated.binding_ids.is_empty());

        assert_eq!(
            engine
                .on_event(event("C", KeyEventPhase::Down, false, 1), ms(30))
                .unwrap()
                .binding_ids,
            vec![id]
        );
    }

    #[test]
    fn uses_physical_keys_and_never_combines_steps_from_different_devices() {
        let id = Uuid::from_u128(4);
        let mut engine = SequenceEngine::new(vec![binding(id, "Ctrl+C, C")]).unwrap();

        engine
            .on_event(event("Ctrl+C", KeyEventPhase::Down, false, 10), ms(0))
            .unwrap();
        assert!(engine
            .on_event(event("C", KeyEventPhase::Down, false, 20), ms(100))
            .unwrap()
            .binding_ids
            .is_empty());

        engine
            .on_event(event("Ctrl+C", KeyEventPhase::Down, false, 20), ms(200))
            .unwrap();
        assert_eq!(
            engine
                .on_event(event("C", KeyEventPhase::Down, false, 20), ms(300))
                .unwrap()
                .binding_ids,
            vec![id]
        );
    }

    #[test]
    fn repeated_chords_overlap_and_equal_matches_keep_registration_order() {
        let first = Uuid::from_u128(5);
        let second = Uuid::from_u128(6);
        let mut engine = SequenceEngine::new(vec![
            binding(first, "Ctrl+C, Ctrl+C"),
            binding(second, "Ctrl+C, Ctrl+C"),
        ])
        .unwrap();

        assert!(engine.on_key(chord("Ctrl+C"), ms(0)).is_empty());
        assert_eq!(engine.on_key(chord("Ctrl+C"), ms(100)), vec![first, second]);
        assert_eq!(engine.on_key(chord("Ctrl+C"), ms(200)), vec![first, second]);
    }

    #[test]
    fn overlapping_prefixes_wait_for_and_emit_only_the_longest_exact_match() {
        let short = Uuid::from_u128(7);
        let long = Uuid::from_u128(8);
        let mut engine = SequenceEngine::new(vec![
            binding(short, "Ctrl+C, C"),
            binding(long, "Ctrl+C, C, V"),
        ])
        .unwrap();

        engine.on_key(chord("Ctrl+C"), ms(0));
        assert!(engine.on_key(chord("C"), ms(100)).is_empty());
        assert_eq!(engine.on_key(chord("V"), ms(200)), vec![long]);
    }

    #[test]
    fn pending_shorter_prefix_fires_after_timeout_or_before_a_mismatch_is_re_evaluated() {
        let short = Uuid::from_u128(11);
        let long = Uuid::from_u128(12);
        let mut engine = SequenceEngine::new(vec![
            binding(short, "Ctrl+C, C"),
            binding(long, "Ctrl+C, C, V"),
        ])
        .unwrap();

        engine.on_key(chord("Ctrl+C"), ms(0));
        assert!(engine.on_key(chord("C"), ms(100)).is_empty());
        assert!(engine.on_time(ms(750)).unwrap().is_empty());
        assert_eq!(engine.on_time(ms(751)).unwrap(), vec![short]);

        engine.on_key(chord("Ctrl+C"), ms(800));
        assert!(engine.on_key(chord("C"), ms(900)).is_empty());
        assert_eq!(engine.on_key(chord("Ctrl+C"), ms(1_000)), vec![short]);
        assert!(engine.on_key(chord("C"), ms(1_100)).is_empty());
        assert_eq!(engine.on_key(chord("Ctrl+X"), ms(1_200)), vec![short]);
    }

    #[test]
    fn suspension_focus_loss_and_explicit_reset_clear_partial_sequences() {
        let id = Uuid::from_u128(9);
        let mut engine = SequenceEngine::new(vec![binding(id, "Ctrl+C, C")]).unwrap();

        engine.on_key(chord("Ctrl+C"), ms(0));
        engine.set_suspended(true);
        assert!(engine.on_key(chord("C"), ms(100)).is_empty());
        engine.set_suspended(false);
        assert!(engine.on_key(chord("C"), ms(200)).is_empty());

        engine.on_key(chord("Ctrl+C"), ms(300));
        engine.on_focus_lost();
        assert!(engine.on_key(chord("C"), ms(400)).is_empty());

        engine.on_key(chord("Ctrl+C"), ms(500));
        engine.reset();
        assert!(engine.on_key(chord("C"), ms(600)).is_empty());
    }

    #[test]
    fn rejects_backward_monotonic_time_and_recovers_from_a_clean_state() {
        let id = Uuid::from_u128(10);
        let mut engine = SequenceEngine::new(vec![binding(id, "Ctrl+C, C")]).unwrap();

        engine
            .on_event(event("Ctrl+C", KeyEventPhase::Down, false, 1), ms(100))
            .unwrap();
        assert_eq!(
            engine.on_event(event("C", KeyEventPhase::Down, false, 1), ms(99)),
            Err(SequenceEngineError::NonMonotonicTimestamp)
        );
        assert!(engine
            .on_event(event("C", KeyEventPhase::Down, false, 1), ms(101))
            .unwrap()
            .binding_ids
            .is_empty());
    }

    #[test]
    fn rejects_more_than_the_bounded_number_of_sequence_patterns() {
        let bindings = (0..=MAX_SEQUENCE_BINDINGS)
            .map(|index| binding(Uuid::from_u128(index as u128 + 100), "Ctrl+C, C"))
            .collect();

        assert_eq!(
            SequenceEngine::new(bindings).unwrap_err(),
            SequenceEngineError::TooManyBindings
        );
    }
}
