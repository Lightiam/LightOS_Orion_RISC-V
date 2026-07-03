//! Per-NCE power state machine: idle → active → turbo.
//!
//! Transitions are stepwise by design (turbo is only reachable from
//! active, so the power rails ramp in order); dropping to idle is
//! allowed from any state. On real hardware each transition is an
//! MMIO doorbell write; on the emulated fallback it's state-only.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PowerState {
    Idle,
    Active,
    Turbo,
}

impl PowerState {
    pub fn as_str(&self) -> &'static str {
        match self {
            PowerState::Idle => "idle",
            PowerState::Active => "active",
            PowerState::Turbo => "turbo",
        }
    }

    pub fn parse(s: &str) -> Option<PowerState> {
        match s.trim() {
            "idle" => Some(PowerState::Idle),
            "active" => Some(PowerState::Active),
            "turbo" => Some(PowerState::Turbo),
            _ => None,
        }
    }

    /// Whether `self -> next` is a legal single transition.
    pub fn can_transition(&self, next: PowerState) -> bool {
        use PowerState::*;
        matches!(
            (*self, next),
            (Idle, Active) | (Active, Turbo) | (Active, Idle) | (Turbo, Active) | (Turbo, Idle)
        )
    }
}

/// Doorbell offsets within an NCE's MMIO window (hardware contract;
/// no-ops on the emulated fallback).
pub const REG_POWER: usize = 0x00;

/// Value written to REG_POWER for each state.
pub fn doorbell_value(state: PowerState) -> u32 {
    match state {
        PowerState::Idle => 0,
        PowerState::Active => 1,
        PowerState::Turbo => 2,
    }
}
