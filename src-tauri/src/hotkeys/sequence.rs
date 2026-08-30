use std::{fmt, time::Duration};

use uuid::Uuid;

use super::{Chord, HotkeyBinding, Trigger};

pub const MAX_SEQUENCE_BINDINGS: usize = 256;
pub const MAX_ACTIVE_DEVICES: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyDevice(pub u64);

impl KeyDevice {
    const UNSPECIFIED: Self = Self(0);
}

impl fmt::Debug for KeyDevice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("KeyDevice(redacted)")
    }
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
struct Pattern {
    id: Uuid,
    steps: Vec<Chord>,
    timeout: Duration,
}

impl Pattern {
    fn from_binding(binding: HotkeyBinding) -> Option<Self> {
        let Trigger::Sequence { steps, timeout_ms } = binding.trigger else {
            return None;
        };
        Some(Self {
            id: binding.id,
            steps,
            timeout: Duration::from_millis(timeout_ms),
        })
    }
}

#[derive(Clone, Debug, Default)]
struct Cursor {
    next_step: usize,
    last_at: Option<Duration>,
}

#[derive(Clone, Debug)]
struct PendingMatch {
    pattern: usize,
    completed_at: Duration,
    timeout: Duration,
}

impl Cursor {
    fn reset(&mut self) {
        self.next_step = 0;
        self.last_at = None;
    }

    fn start_if_first(&mut self, pattern: &Pattern, chord: Chord, at: Duration) {
        if pattern.steps.first() == Some(&chord) {
            self.next_step = 1;
            self.last_at = Some(at);
        }
    }

    fn advance(&mut self, pattern: &Pattern, chord: Chord, at: Duration) -> bool {
        if self.next_step == 0 {
            self.start_if_first(pattern, chord, at);
            return false;
        }

        let expired = self
            .last_at
            .is_some_and(|last_at| at.saturating_sub(last_at) > pattern.timeout);
        if expired {
            self.reset();
            self.start_if_first(pattern, chord, at);
            return false;
        }

        if pattern.steps.get(self.next_step) != Some(&chord) {
            self.reset();
            self.start_if_first(pattern, chord, at);
            return false;
        }

        self.next_step += 1;
        self.last_at = Some(at);
        if self.next_step != pattern.steps.len() {
            return false;
        }

        self.reset();
        self.start_if_first(pattern, chord, at);
        true
    }

    fn expire(&mut self, pattern: &Pattern, at: Duration) {
        if self
            .last_at
            .is_some_and(|last_at| at.saturating_sub(last_at) > pattern.timeout)
        {
            self.reset();
        }
    }
}

#[derive(Debug)]
struct DeviceState {
    device: KeyDevice,
    cursors: Vec<Cursor>,
    pending: Vec<PendingMatch>,
    last_seen_at: Duration,
}

impl DeviceState {
    fn new(device: KeyDevice, pattern_count: usize, at: Duration) -> Self {
        Self {
            device,
            cursors: vec![Cursor::default(); pattern_count],
            pending: Vec::new(),
            last_seen_at: at,
        }
    }

    fn is_idle(&self) -> bool {
        self.pending.is_empty() && self.cursors.iter().all(|cursor| cursor.next_step == 0)
    }
}

/// Deterministic sequence recognizer owned by one serializing controller.
///
/// The type is `Send`, so the controller may move it to a dedicated input
/// thread, but callers must serialize `on_event` and `on_time` on that owner.
/// Device identities are opaque numeric values and are never logged. When the
/// active-device cap is reached, the least-recently observed partial state is
/// discarded without firing before a new device is admitted.
#[derive(Debug)]
pub struct SequenceEngine {
    patterns: Vec<Pattern>,
    devices: Vec<DeviceState>,
    last_event_at: Option<Duration>,
    suspended: bool,
}

impl SequenceEngine {
    /// Consumes bindings by value and stops at the first item over the bound.
    /// Passing a `Vec` uses its owning iterator and does not copy its entries.
    pub fn new<I>(bindings: I) -> Result<Self, SequenceEngineError>
    where
        I: IntoIterator<Item = HotkeyBinding>,
    {
        let mut patterns = Vec::new();
        for (index, binding) in bindings.into_iter().enumerate() {
            if index == MAX_SEQUENCE_BINDINGS {
                return Err(SequenceEngineError::TooManyBindings);
            }
            if let Some(pattern) = Pattern::from_binding(binding) {
                patterns.push(pattern);
            }
        }
        Ok(Self {
            patterns,
            devices: Vec::new(),
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
            self.clear_devices();
            return Err(SequenceEngineError::NonMonotonicTimestamp);
        }
        self.last_event_at = Some(at);

        if self.suspended || event.phase != KeyEventPhase::Down || event.repeat {
            return Ok(SequenceOutcome::pass_through(Vec::new()));
        }

        let device_index = match self
            .devices
            .iter()
            .position(|state| state.device == event.device)
        {
            Some(index) => index,
            None => {
                if !self
                    .patterns
                    .iter()
                    .any(|pattern| pattern.steps.first() == Some(&event.chord))
                {
                    return Ok(SequenceOutcome::pass_through(Vec::new()));
                }
                if self.devices.len() == MAX_ACTIVE_DEVICES {
                    let least_recent = self
                        .devices
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, state)| state.last_seen_at)
                        .map(|(index, _)| index)
                        .expect("the active-device cap is nonzero");
                    self.devices.remove(least_recent);
                }
                self.devices
                    .push(DeviceState::new(event.device, self.patterns.len(), at));
                self.devices.len() - 1
            }
        };

        let binding_ids = process_device(
            &self.patterns,
            &mut self.devices[device_index],
            event.chord,
            at,
        );
        self.devices[device_index].last_seen_at = at;
        if self.devices[device_index].is_idle() {
            self.devices.remove(device_index);
        }
        Ok(SequenceOutcome::pass_through(binding_ids))
    }

    pub fn on_time(&mut self, at: Duration) -> Result<Vec<Uuid>, SequenceEngineError> {
        if self.last_event_at.is_some_and(|last_at| at < last_at) {
            self.clear_devices();
            return Err(SequenceEngineError::NonMonotonicTimestamp);
        }
        self.last_event_at = Some(at);

        let mut binding_ids = Vec::new();
        for state in &mut self.devices {
            for (cursor, pattern) in state.cursors.iter_mut().zip(&self.patterns) {
                cursor.expire(pattern, at);
            }
            let expired = take_expired_pending(&self.patterns, state, at);
            for id in expired {
                push_unique(&mut binding_ids, id);
            }
        }
        self.devices.retain(|state| !state.is_idle());
        Ok(binding_ids)
    }

    pub fn set_suspended(&mut self, suspended: bool) {
        self.suspended = suspended;
        self.clear_devices();
    }

    pub fn on_focus_lost(&mut self) {
        self.clear_devices();
    }

    pub fn reset(&mut self) {
        self.clear_devices();
        self.last_event_at = None;
    }

    fn clear_devices(&mut self) {
        self.devices.clear();
    }
}

fn process_device(
    patterns: &[Pattern],
    state: &mut DeviceState,
    chord: Chord,
    at: Duration,
) -> Vec<Uuid> {
    let mut binding_ids = take_expired_pending(patterns, state, at);
    for (cursor, pattern) in state.cursors.iter_mut().zip(patterns) {
        cursor.expire(pattern, at);
    }

    let mut completed = Vec::new();
    for (index, (cursor, pattern)) in state.cursors.iter_mut().zip(patterns).enumerate() {
        if cursor.advance(pattern, chord, at) {
            completed.push(index);
        }
    }

    let previous_pending = std::mem::take(&mut state.pending);
    for pending in previous_pending {
        let extended_by_completion = completed
            .iter()
            .any(|&matched| is_strict_prefix(&patterns[pending.pattern], &patterns[matched]));
        if extended_by_completion {
            continue;
        }
        if has_active_extension(patterns, state, pending.pattern) {
            state.pending.push(pending);
        } else {
            push_unique(&mut binding_ids, patterns[pending.pattern].id);
        }
    }

    for &matched in &completed {
        let shadowed_by_longer = completed
            .iter()
            .any(|&other| is_strict_prefix(&patterns[matched], &patterns[other]));
        if shadowed_by_longer {
            continue;
        }
        if has_active_extension(patterns, state, matched) {
            let timeout = patterns
                .iter()
                .enumerate()
                .filter(|(index, _)| is_strict_prefix(&patterns[matched], &patterns[*index]))
                .map(|(_, pattern)| pattern.timeout)
                .max()
                .unwrap_or_default();
            state.pending.push(PendingMatch {
                pattern: matched,
                completed_at: at,
                timeout,
            });
        } else {
            push_unique(&mut binding_ids, patterns[matched].id);
        }
    }
    binding_ids
}

fn has_active_extension(patterns: &[Pattern], state: &DeviceState, matched: usize) -> bool {
    state
        .cursors
        .iter()
        .zip(patterns)
        .any(|(cursor, candidate)| {
            is_strict_prefix(&patterns[matched], candidate)
                && cursor.next_step == patterns[matched].steps.len()
        })
}

fn is_strict_prefix(prefix: &Pattern, candidate: &Pattern) -> bool {
    candidate.steps.len() > prefix.steps.len() && candidate.steps.starts_with(&prefix.steps)
}

fn take_expired_pending(patterns: &[Pattern], state: &mut DeviceState, at: Duration) -> Vec<Uuid> {
    let mut expired = Vec::new();
    state.pending.retain(|pending| {
        if at.saturating_sub(pending.completed_at) > pending.timeout {
            push_unique(&mut expired, patterns[pending.pattern].id);
            false
        } else {
            true
        }
    });
    expired
}

fn push_unique(ids: &mut Vec<Uuid>, id: Uuid) {
    if !ids.contains(&id) {
        ids.push(id);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use uuid::Uuid;

    use super::{
        KeyDevice, KeyEvent, KeyEventPhase, KeyPropagation, SequenceEngine, SequenceEngineError,
        MAX_ACTIVE_DEVICES, MAX_SEQUENCE_BINDINGS,
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
    fn overlapping_pending_and_new_completion_emit_each_binding_id_once() {
        let short = Uuid::from_u128(13);
        let long = Uuid::from_u128(14);
        let mut engine = SequenceEngine::new(vec![
            binding(short, "Ctrl+A, Ctrl+A"),
            binding(long, "Ctrl+A, Ctrl+A, Ctrl+B"),
        ])
        .unwrap();

        assert!(engine.on_key(chord("Ctrl+A"), ms(0)).is_empty());
        assert!(engine.on_key(chord("Ctrl+A"), ms(100)).is_empty());
        assert_eq!(engine.on_key(chord("Ctrl+A"), ms(200)), vec![short]);
    }

    #[test]
    fn devices_keep_independent_cursors_pending_matches_and_deadlines() {
        let first_short = Uuid::from_u128(15);
        let first_long = Uuid::from_u128(16);
        let second_short = Uuid::from_u128(17);
        let second_long = Uuid::from_u128(18);
        let mut engine = SequenceEngine::new(vec![
            binding(first_short, "Ctrl+A, A"),
            binding(first_long, "Ctrl+A, A, B"),
            binding(second_short, "Ctrl+C, C"),
            binding(second_long, "Ctrl+C, C, V"),
        ])
        .unwrap();

        engine
            .on_event(event("Ctrl+A", KeyEventPhase::Down, false, 1), ms(0))
            .unwrap();
        engine
            .on_event(event("Ctrl+C", KeyEventPhase::Down, false, 2), ms(0))
            .unwrap();
        assert!(engine
            .on_event(event("A", KeyEventPhase::Down, false, 1), ms(100))
            .unwrap()
            .binding_ids
            .is_empty());
        assert!(engine
            .on_event(event("C", KeyEventPhase::Down, false, 2), ms(100))
            .unwrap()
            .binding_ids
            .is_empty());

        assert_eq!(
            engine
                .on_event(event("B", KeyEventPhase::Down, false, 1), ms(200))
                .unwrap()
                .binding_ids,
            vec![first_long]
        );
        assert_eq!(engine.on_time(ms(750)).unwrap(), Vec::<Uuid>::new());
        assert_eq!(engine.on_time(ms(751)).unwrap(), vec![second_short]);
    }

    #[test]
    fn timeout_flushes_all_devices_in_first_observed_order() {
        let first = Uuid::from_u128(19);
        let first_long = Uuid::from_u128(20);
        let second = Uuid::from_u128(21);
        let second_long = Uuid::from_u128(22);
        let mut engine = SequenceEngine::new(vec![
            binding(first, "Ctrl+A, A"),
            binding(first_long, "Ctrl+A, A, B"),
            binding(second, "Ctrl+C, C"),
            binding(second_long, "Ctrl+C, C, V"),
        ])
        .unwrap();

        engine
            .on_event(event("Ctrl+A", KeyEventPhase::Down, false, 9), ms(0))
            .unwrap();
        engine
            .on_event(event("Ctrl+C", KeyEventPhase::Down, false, 3), ms(0))
            .unwrap();
        engine
            .on_event(event("A", KeyEventPhase::Down, false, 9), ms(100))
            .unwrap();
        engine
            .on_event(event("C", KeyEventPhase::Down, false, 3), ms(100))
            .unwrap();

        assert_eq!(engine.on_time(ms(751)).unwrap(), vec![first, second]);
    }

    #[test]
    fn active_device_limit_evicts_the_least_recent_state_without_firing_it() {
        let id = Uuid::from_u128(23);
        let mut engine = SequenceEngine::new(vec![binding(id, "Ctrl+A, A")]).unwrap();

        for device in 1..=MAX_ACTIVE_DEVICES as u64 {
            engine
                .on_event(
                    event("Ctrl+A", KeyEventPhase::Down, false, device),
                    ms(device),
                )
                .unwrap();
        }
        engine
            .on_event(
                event(
                    "Ctrl+A",
                    KeyEventPhase::Down,
                    false,
                    MAX_ACTIVE_DEVICES as u64 + 1,
                ),
                ms(MAX_ACTIVE_DEVICES as u64 + 1),
            )
            .unwrap();

        assert!(engine
            .on_event(
                event("A", KeyEventPhase::Down, false, 1),
                ms(MAX_ACTIVE_DEVICES as u64 + 2),
            )
            .unwrap()
            .binding_ids
            .is_empty());
        assert_eq!(
            engine
                .on_event(
                    event("A", KeyEventPhase::Down, false, MAX_ACTIVE_DEVICES as u64,),
                    ms(MAX_ACTIVE_DEVICES as u64 + 3),
                )
                .unwrap()
                .binding_ids,
            vec![id]
        );
    }

    #[test]
    fn suspension_focus_loss_and_explicit_reset_clear_partial_sequences() {
        let id = Uuid::from_u128(9);
        let mut engine = SequenceEngine::new(vec![binding(id, "Ctrl+C, C")]).unwrap();

        engine
            .on_event(event("Ctrl+C", KeyEventPhase::Down, false, 1), ms(0))
            .unwrap();
        engine
            .on_event(event("Ctrl+C", KeyEventPhase::Down, false, 2), ms(0))
            .unwrap();
        engine.set_suspended(true);
        engine.set_suspended(false);
        for device in [1, 2] {
            assert!(engine
                .on_event(event("C", KeyEventPhase::Down, false, device), ms(100))
                .unwrap()
                .binding_ids
                .is_empty());
        }

        engine
            .on_event(event("Ctrl+C", KeyEventPhase::Down, false, 1), ms(200))
            .unwrap();
        engine
            .on_event(event("Ctrl+C", KeyEventPhase::Down, false, 2), ms(200))
            .unwrap();
        engine.on_focus_lost();
        for device in [1, 2] {
            assert!(engine
                .on_event(event("C", KeyEventPhase::Down, false, device), ms(300))
                .unwrap()
                .binding_ids
                .is_empty());
        }

        engine.on_key(chord("Ctrl+C"), ms(400));
        engine.reset();
        assert!(engine.on_key(chord("C"), ms(500)).is_empty());
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
        let bindings: Vec<_> = (0..=MAX_SEQUENCE_BINDINGS)
            .map(|index| binding(Uuid::from_u128(index as u128 + 100), "Ctrl+C, C"))
            .collect();

        assert_eq!(
            SequenceEngine::new(bindings).unwrap_err(),
            SequenceEngineError::TooManyBindings
        );
    }

    #[test]
    fn constructor_stops_consuming_at_the_first_binding_over_the_limit() {
        use std::{cell::Cell, rc::Rc};

        struct CountingBindings {
            yielded: Rc<Cell<usize>>,
        }

        impl Iterator for CountingBindings {
            type Item = HotkeyBinding;

            fn next(&mut self) -> Option<Self::Item> {
                let index = self.yielded.get();
                self.yielded.set(index + 1);
                Some(binding(Uuid::from_u128(index as u128 + 1_000), "Ctrl+C, C"))
            }
        }

        let yielded = Rc::new(Cell::new(0));
        let result = SequenceEngine::new(CountingBindings {
            yielded: yielded.clone(),
        });

        assert_eq!(result.unwrap_err(), SequenceEngineError::TooManyBindings);
        assert_eq!(yielded.get(), MAX_SEQUENCE_BINDINGS + 1);
    }

    #[test]
    fn engine_is_send_for_a_single_serializing_controller_owner() {
        fn assert_send<T: Send>() {}
        assert_send::<SequenceEngine>();
    }

    #[test]
    fn debug_output_never_exposes_device_identifiers() {
        let secret_device = 9_876_543_210_u64;
        let mut engine =
            SequenceEngine::new(vec![binding(Uuid::from_u128(24), "Ctrl+C, C")]).unwrap();
        let observed = event("Ctrl+C", KeyEventPhase::Down, false, secret_device);
        engine.on_event(observed, ms(0)).unwrap();

        assert!(!format!("{observed:?}").contains(&secret_device.to_string()));
        assert!(!format!("{engine:?}").contains(&secret_device.to_string()));
    }
}
