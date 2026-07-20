//! Shared helpers for NetFlow wire-field conversions.

pub(crate) const NANOS_PER_MILLI: u64 = 1_000_000;

pub(crate) fn uptime_millis_to_netflow_u32(value: i64) -> u32 {
    if value < 0 {
        return 0;
    }

    value as u32
}

pub(crate) fn saturating_u64_to_netflow_u32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

pub(crate) fn boot_nanos_to_netflow_millis(value: u64) -> u32 {
    (value / NANOS_PER_MILLI) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tracing::{Event, Level, Subscriber};
    use tracing_subscriber::{
        layer::{Context, Layer, SubscriberExt},
        registry::LookupSpan,
    };

    struct WarningCounter {
        count: Arc<AtomicUsize>,
    }

    impl<S> Layer<S> for WarningCounter
    where
        S: Subscriber + for<'span> LookupSpan<'span>,
    {
        fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
            if *event.metadata().level() == Level::WARN {
                self.count.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    #[test]
    fn uptime_millis_wraps_like_netflow_wire_fields() {
        assert_eq!(uptime_millis_to_netflow_u32(i64::from(u32::MAX)), u32::MAX);
        assert_eq!(uptime_millis_to_netflow_u32(i64::from(u32::MAX) + 1), 0);
        assert_eq!(uptime_millis_to_netflow_u32(-1), 0);
    }

    #[test]
    fn boundary_conversions_do_not_emit_warnings() {
        let warning_count = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry().with(WarningCounter {
            count: Arc::clone(&warning_count),
        });

        tracing::subscriber::with_default(subscriber, || {
            assert_eq!(uptime_millis_to_netflow_u32(-1), 0);
            assert_eq!(uptime_millis_to_netflow_u32(i64::from(u32::MAX) + 1), 0);
            assert_eq!(
                saturating_u64_to_netflow_u32(u64::from(u32::MAX) + 1),
                u32::MAX
            );
            assert_eq!(
                boot_nanos_to_netflow_millis((u64::from(u32::MAX) + 1) * NANOS_PER_MILLI),
                0
            );
        });

        assert_eq!(warning_count.load(Ordering::Relaxed), 0);
    }
}
