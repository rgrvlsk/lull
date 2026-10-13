//! Pulses and periodic traffic (contract §5, ADR-0010): groups one client's traffic to one
//! server into pulses, learns the trains that pollers make, and reports only the pulses
//! that are not periodic traffic.

use std::collections::{HashMap, VecDeque};

use crate::model::ServerId;
use crate::traits::Instant;

const SLOT_MS: u64 = 1_000;
/// A pulse spans at most this many consecutive slots.
const PULSE_SLOTS: u64 = 2;
const MIN_PERIOD_MS: u64 = 3_000;
const MAX_PERIOD_MS: u64 = 600_000;
const MIN_TOLERANCE_MS: u64 = 1_000;
const SIZE_SLACK: u64 = 32;
/// Pulses that confirm a train.
const CONFIRMED: u32 = 4;
/// A pulse may continue a train at the next expected instant or one of the two after it;
/// the train is dropped once all of them have passed.
const MISSES: u64 = 3;
const MAX_TRAINS: usize = 16;
const HISTORY_MS: u64 = 600_000;
/// Safety bound only: pulses are at least two slots apart, so ten minutes hold 300.
const MAX_HISTORY: usize = 512;

/// One client of one server: the client pid, `None` for remote or unknown clients.
pub type Key = (ServerId, Option<u32>);

/// What a backend saw for a key at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    /// A new connection.
    Opened,
    /// Bytes moved since the previous reading.
    Bytes(u64),
    /// A new connection closed before its counters were read: the pulse size is unknown.
    Unread,
}

/// Every key's open pulse, trains and recent pulses, all in one boot session.
#[derive(Debug, Default)]
pub struct Periodic {
    session: Option<String>,
    keys: HashMap<Key, KeyState>,
}

#[derive(Debug, Default)]
struct KeyState {
    open: Option<Pulse>,
    /// Pulses closed by a later observation, judged at the next `settle`.
    closed_early: Vec<Pulse>,
    trains: Vec<Train>,
    /// Instant (ms) and size of recent pulses that continued no confirmed train.
    recent: VecDeque<(u64, u64)>,
}

#[derive(Debug)]
struct Pulse {
    first_slot: u64,
    at: Instant,
    bytes: u64,
    unknown: bool,
    /// Whether this pulse may fire `traffic` at all: (a) a new connection, or (b) bytes
    /// from a client that is not a covered browser.
    can_fire: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Train {
    /// Mean interval since `first`: single intervals carry the backend's timing error, the
    /// mean does not accumulate it.
    period: u64,
    first: u64,
    /// Periods from `first` to `last`, counting skipped ones.
    beats: u64,
    last: u64,
    size: u64,
    pulses: u32,
}

impl Periodic {
    /// Records one observation; `can_fire` says whether it alone may fire `traffic`.
    pub fn observe(&mut self, key: Key, at: &Instant, seen: Seen, can_fire: bool) {
        self.enter(at);
        let state = self.keys.entry(key).or_default();
        let slot = at.uptime_ms / SLOT_MS;
        if let Seen::Unread = seen {
            // Never makes a slot busy by itself; it taints the pulse its connection is in.
            if let Some(open) = state.open.as_mut() {
                open.unknown = true;
            }
            return;
        }
        if matches!(seen, Seen::Bytes(0)) {
            return;
        }
        let joins = state
            .open
            .as_ref()
            .is_some_and(|open| slot < open.first_slot + PULSE_SLOTS);
        if !joins {
            // The previous pulse is complete; it is judged at the next `settle`, in order.
            state.closed_early.extend(state.open.take());
            state.open = Some(Pulse {
                first_slot: slot,
                at: at.clone(),
                bytes: 0,
                unknown: false,
                can_fire: false,
            });
        }
        let open = state.open.as_mut().expect("open pulse");
        if let Seen::Bytes(n) = seen {
            open.bytes = open.bytes.saturating_add(n);
        }
        open.can_fire |= can_fire;
    }

    /// Closes every pulse whose last slot has passed by `now`, judges it, and returns the
    /// instants of the pulses that fire `traffic`, per server.
    pub fn settle(&mut self, now: &Instant) -> Vec<(ServerId, Instant)> {
        self.enter(now);
        let now_ms = now.uptime_ms;
        let now_slot = now_ms / SLOT_MS;
        let mut fired = Vec::new();
        for (key, state) in &mut self.keys {
            let mut closed = std::mem::take(&mut state.closed_early);
            if state
                .open
                .as_ref()
                .is_some_and(|open| now_slot >= open.first_slot + PULSE_SLOTS)
            {
                closed.extend(state.open.take());
            }
            for pulse in closed {
                let size = (!pulse.unknown).then_some(pulse.bytes);
                let periodic = state.judge(pulse.at.uptime_ms, size);
                if !periodic && pulse.can_fire {
                    fired.push((key.0.clone(), pulse.at));
                }
            }
            state.expire(now_ms);
        }
        self.keys.retain(|_, state| !state.is_empty());
        fired
    }

    /// Instants from another boot session are not comparable: start over.
    fn enter(&mut self, at: &Instant) {
        if self.session.as_deref() != Some(at.boot_session.as_str()) {
            self.session = Some(at.boot_session.clone());
            self.keys.clear();
        }
    }
}

impl KeyState {
    /// Feeds one pulse to the trains; `true` when it is periodic traffic.
    fn judge(&mut self, t: u64, size: Option<u64>) -> bool {
        self.expire(t);
        let Some(size) = size else {
            return false;
        };
        let mut periodic = false;
        let mut continued = false;
        for train in &mut self.trains {
            if let Some(beats) = train.continues_with(t, size) {
                continued = true;
                let was_confirmed = train.pulses >= CONFIRMED;
                train.beats += beats;
                train.period = t.saturating_sub(train.first) / train.beats;
                train.pulses += 1;
                train.last = t;
                train.size = size;
                periodic |= was_confirmed || train.pulses >= CONFIRMED;
            }
        }
        if !continued {
            // Oldest partners first, so that past the cap the longest candidates go first.
            let candidates: Vec<Train> = self
                .recent
                .iter()
                .filter_map(|&(rt, rs)| {
                    let period = t.checked_sub(rt)?;
                    ((MIN_PERIOD_MS..=MAX_PERIOD_MS).contains(&period) && sizes_match(rs, size))
                        .then_some(Train {
                            period,
                            first: rt,
                            beats: 1,
                            last: t,
                            size,
                            pulses: 2,
                        })
                })
                .collect();
            for train in candidates {
                self.trains.push(train);
                if self.trains.len() > MAX_TRAINS {
                    let oldest = (0..self.trains.len())
                        .min_by_key(|&i| self.trains[i].last)
                        .expect("trains");
                    self.trains.remove(oldest);
                }
            }
        }
        if !periodic {
            self.recent.push_back((t, size));
            if self.recent.len() > MAX_HISTORY {
                self.recent.pop_front();
            }
        }
        periodic
    }

    /// Drops trains past their last chance or whose mean period left the allowed range,
    /// and pulses older than the history window.
    fn expire(&mut self, now_ms: u64) {
        self.trains.retain(|train| {
            (MIN_PERIOD_MS..=MAX_PERIOD_MS).contains(&train.period)
                && now_ms <= train.last + MISSES * train.period + tolerance(train.period)
        });
        while self
            .recent
            .front()
            .is_some_and(|&(t, _)| now_ms.saturating_sub(t) > HISTORY_MS)
        {
            self.recent.pop_front();
        }
    }

    fn is_empty(&self) -> bool {
        self.open.is_none()
            && self.closed_early.is_empty()
            && self.trains.is_empty()
            && self.recent.is_empty()
    }
}

impl Train {
    /// How many periods after the last pulse `t` lands, when it continues this train.
    fn continues_with(&self, t: u64, size: u64) -> Option<u64> {
        if !sizes_match(self.size, size) {
            return None;
        }
        (1..=MISSES).find(|&k| t.abs_diff(self.last + k * self.period) <= tolerance(self.period))
    }
}

fn tolerance(period: u64) -> u64 {
    MIN_TOLERANCE_MS.max(period / 10)
}

fn sizes_match(a: u64, b: u64) -> bool {
    a.abs_diff(b) <= SIZE_SLACK.max(a.max(b) / 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> Instant {
        Instant {
            uptime_ms: ms,
            boot_session: "boot-A".into(),
        }
    }

    fn key(pid: u32) -> Key {
        ("100-1".into(), Some(pid))
    }

    type Obs = (u64, Key, Seen, bool);

    /// A page polling on a timer over keep-alive: `count` pulses of `size` bytes.
    fn poller(pid: u32, start: u64, period: u64, count: u64, size: u64) -> Vec<Obs> {
        (0..count)
            .map(|i| (start + i * period, key(pid), Seen::Bytes(size), true))
            .collect()
    }

    /// Feeds observations in time order, settling every 5 s like the daemon's scans, and
    /// returns the instants that fired `traffic`.
    fn run(periodic: &mut Periodic, obs: &[Obs]) -> Vec<u64> {
        let mut obs = obs.to_vec();
        obs.sort_by_key(|o| o.0);
        let end = obs.last().map_or(0, |o| o.0) + 10_000;
        let mut fired = Vec::new();
        let mut next = obs.into_iter().peekable();
        let mut scan = 5_000;
        while scan <= end {
            while let Some((t, key, seen, can_fire)) = next.next_if(|o| o.0 <= scan) {
                periodic.observe(key, &at(t), seen, can_fire);
            }
            fired.extend(
                periodic
                    .settle(&at(scan))
                    .into_iter()
                    .map(|(_, a)| a.uptime_ms),
            );
            scan += 5_000;
        }
        fired.sort_unstable();
        fired
    }

    #[test]
    fn a_poller_fires_for_its_first_three_pulses_only() {
        let mut p = Periodic::default();
        assert_eq!(
            run(&mut p, &poller(7, 0, 10_000, 60, 400)),
            vec![0, 10_000, 20_000]
        );
    }

    #[test]
    fn a_poll_that_opens_a_connection_each_time_is_one_rhythm() {
        // Node closes keep-alive connections after 5 s, so a 10 s poll opens a new
        // connection every time and its bytes are read up to a second later.
        let obs: Vec<Obs> = (0..30)
            .flat_map(|i| {
                let t = 2_500 + i * 10_000;
                [
                    (t, key(7), Seen::Opened, true),
                    (t + 600, key(7), Seen::Bytes(1_200), true),
                ]
            })
            .collect();
        let mut p = Periodic::default();
        assert_eq!(run(&mut p, &obs), vec![2_500, 12_500, 22_500]);
    }

    #[test]
    fn a_poll_read_on_the_sample_grid_stays_one_train() {
        // Bytes carry the instant of the sample that read them, on a 250 ms grid, so the
        // intervals of a poll with a little timer jitter wobble by up to a grid step or two;
        // sizes vary a little too.
        for period in [3_000u64, 5_020, 9_980, 10_000, 10_300, 10_970, 60_000] {
            let obs: Vec<Obs> = (0..200)
                .map(|i| {
                    let jitter: u64 = [0, 40, 15, 70, 5, 55][i as usize % 6];
                    let sent = 250 + i * period + jitter;
                    let read = sent.div_ceil(250) * 250;
                    let size = if i % 3 == 0 { 400 } else { 430 };
                    (read, key(7), Seen::Bytes(size), true)
                })
                .collect();
            let mut p = Periodic::default();
            let fired = run(&mut p, &obs);
            assert_eq!(fired.len(), 3, "period {period}: {fired:?}");
        }
    }

    #[test]
    fn hidden_tabs_sharing_one_network_process_form_separate_trains() {
        for second_size in [900, 400] {
            let mut obs = poller(7, 0, 10_000, 40, 400);
            obs.extend(poller(7, 3_000, 10_000, 40, second_size));
            let mut p = Periodic::default();
            let fired = run(&mut p, &obs);
            assert!(fired.len() <= 6, "size {second_size}: {fired:?}");
            assert!(
                fired.iter().all(|&t| t < 60_000),
                "size {second_size}: {fired:?}"
            );
        }
    }

    #[test]
    fn a_reload_or_a_save_amid_a_poll_fires() {
        let mut obs = poller(7, 0, 10_000, 30, 400);
        obs.push((95_500, key(7), Seen::Opened, true));
        obs.push((95_700, key(7), Seen::Bytes(48_000), true));
        // An HMR update on the same rhythm but another size.
        obs.push((150_000, key(7), Seen::Bytes(2_000), true));
        let mut p = Periodic::default();
        assert_eq!(run(&mut p, &obs), vec![0, 10_000, 20_000, 95_500, 150_000]);
    }

    #[test]
    fn continuous_use_keeps_firing() {
        // A person clicking around: irregular timing and sizes.
        let clicks: Vec<Obs> = [0, 4_000, 11_000, 15_000, 27_000, 31_000, 44_000, 46_000]
            .iter()
            .zip([3_100, 800, 12_000, 800, 5_400, 800, 950, 22_000])
            .map(|(&t, size)| (t, key(7), Seen::Bytes(size), true))
            .collect();
        let mut p = Periodic::default();
        assert_eq!(run(&mut p, &clicks).len(), clicks.len());
    }

    #[test]
    fn pulses_of_unknown_size_always_fire_and_never_join_a_train() {
        let obs: Vec<Obs> = (0..20)
            .flat_map(|i| {
                let t = i * 10_000;
                [
                    (t, key(7), Seen::Opened, true),
                    (t + 50, key(7), Seen::Unread, false),
                ]
            })
            .collect();
        let mut p = Periodic::default();
        assert_eq!(run(&mut p, &obs).len(), 20);
    }

    #[test]
    fn a_pulse_spans_at_most_two_slots() {
        let obs = vec![
            (100, key(7), Seen::Opened, true),
            (900, key(7), Seen::Bytes(10), true),
            (1_500, key(7), Seen::Bytes(10), true),
            (2_100, key(7), Seen::Bytes(10), true),
        ];
        let mut p = Periodic::default();
        assert_eq!(run(&mut p, &obs), vec![100, 2_100]);
        // Zero bytes and a stray `Unread` never make a slot busy.
        let obs = vec![
            (100, key(7), Seen::Bytes(0), true),
            (200, key(7), Seen::Unread, false),
        ];
        let mut p = Periodic::default();
        assert!(run(&mut p, &obs).is_empty());
    }

    #[test]
    fn a_changed_period_is_learned_again() {
        // Chromium throttles a hidden tab's timers to once a minute after five minutes.
        let mut obs = poller(7, 0, 10_000, 30, 400);
        obs.extend(poller(7, 340_000, 60_000, 15, 400));
        let mut p = Periodic::default();
        let fired = run(&mut p, &obs);
        assert_eq!(&fired[..3], &[0, 10_000, 20_000]);
        assert!(fired.len() <= 6, "{fired:?}");
        assert!(fired.iter().all(|&t| t < 340_000 + 4 * 60_000), "{fired:?}");
    }

    #[test]
    fn a_train_is_dropped_after_three_missed_pulses() {
        let mut obs = poller(7, 0, 10_000, 10, 400);
        // Two missed pulses: the train still holds.
        obs.extend(poller(7, 120_000, 10_000, 5, 400));
        let mut p = Periodic::default();
        assert_eq!(run(&mut p, &obs).len(), 3);
        // Silent for longer: the train is gone and the next pulse fires again.
        let mut obs = poller(7, 0, 10_000, 10, 400);
        obs.extend(poller(7, 135_000, 10_000, 5, 400));
        let mut p = Periodic::default();
        let fired = run(&mut p, &obs);
        assert_eq!(&fired[..4], &[0, 10_000, 20_000, 135_000], "{fired:?}");
    }

    #[test]
    fn pulses_that_cannot_fire_still_teach_the_train() {
        // A covered browser's hidden tab polls over keep-alive (bytes cannot fire), then
        // its connection expires and the same poll opens a new one (which could).
        let mut obs: Vec<Obs> = (0..6)
            .map(|i| (i * 10_000, key(7), Seen::Bytes(400), false))
            .collect();
        obs.push((60_000, key(7), Seen::Opened, true));
        obs.push((60_300, key(7), Seen::Bytes(400), false));
        let mut p = Periodic::default();
        assert!(run(&mut p, &obs).is_empty());
    }

    #[test]
    fn clients_are_judged_separately_and_state_is_freed() {
        let mut obs = poller(7, 0, 10_000, 10, 400);
        obs.extend(poller(8, 5_000, 10_000, 10, 400));
        let mut p = Periodic::default();
        assert_eq!(run(&mut p, &obs).len(), 6);
        // Ten minutes after the last pulse nothing is kept.
        p.settle(&at(800_000));
        assert!(p.keys.is_empty());
    }

    #[test]
    fn a_new_boot_session_starts_over() {
        let mut p = Periodic::default();
        run(&mut p, &poller(7, 0, 10_000, 10, 400));
        assert!(!p.keys.is_empty());
        let other = Instant {
            uptime_ms: 1_000,
            boot_session: "boot-B".into(),
        };
        assert!(p.settle(&other).is_empty());
        assert!(p.keys.is_empty());
    }
}
