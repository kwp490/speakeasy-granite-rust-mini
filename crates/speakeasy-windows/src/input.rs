use std::thread;
use std::time::{Duration, Instant};

use speakeasy_domain::DeliveryRefusal;

const MODIFIER_POLL_INTERVAL: Duration = Duration::from_millis(5);

#[cfg(windows)]
fn activation_modifiers_held() -> bool {
    use winsafe::co::VK;
    [VK::CONTROL, VK::MENU, VK::SHIFT, VK::LWIN, VK::RWIN]
        .into_iter()
        .any(winsafe::GetAsyncKeyState)
}

#[must_use]
pub fn activation_modifiers_released() -> bool {
    #[cfg(windows)]
    {
        !activation_modifiers_held()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn wait_for_modifiers_with(
    deadline: Instant,
    mut modifiers_held: impl FnMut() -> bool,
    mut wait: impl FnMut(),
) -> Result<(), DeliveryRefusal> {
    loop {
        if !modifiers_held() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(DeliveryRefusal::ModifierHeld);
        }
        wait();
    }
}

/// Waits off the coordinator thread for activation modifiers to be released.
///
/// # Errors
///
/// Returns [`DeliveryRefusal::ModifierHeld`] at the deadline, or
/// [`DeliveryRefusal::Unsupported`] outside Windows.
pub fn wait_for_activation_modifiers(deadline: Instant) -> Result<(), DeliveryRefusal> {
    #[cfg(windows)]
    {
        wait_for_modifiers_with(deadline, activation_modifiers_held, || {
            thread::sleep(MODIFIER_POLL_INTERVAL);
        })
    }
    #[cfg(not(windows))]
    {
        let _ = deadline;
        Err(DeliveryRefusal::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_wait_completes_after_release_without_unbounded_delay() {
        let mut polls = 0;
        let result = wait_for_modifiers_with(
            Instant::now() + Duration::from_secs(1),
            || {
                polls += 1;
                polls < 3
            },
            || {},
        );
        assert_eq!(result, Ok(()));
        assert_eq!(polls, 3);
    }

    #[test]
    fn modifier_wait_refuses_at_deadline() {
        let result = wait_for_modifiers_with(Instant::now(), || true, || {});
        assert_eq!(result, Err(DeliveryRefusal::ModifierHeld));
    }
}
