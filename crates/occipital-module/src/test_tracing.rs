//! Field-only tracing observers for parallel unit tests. The dispatcher stays
//! installed even when no test is observing events, so first use of a callsite
//! on another thread cannot cache the interest of an absent local subscriber.

use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock,
    },
};

pub(crate) trait Capture: Send + Sync + 'static {
    fn event(&self, event: &tracing::Event<'_>);
    fn new_span(&self, _: &tracing::span::Attributes<'_>) {}
    fn record(&self, _: &tracing::span::Record<'_>) {}
}

thread_local! {
    static ACTIVE: RefCell<Vec<Arc<dyn Capture>>> = RefCell::new(Vec::new());
}

/// Observes events and span fields emitted on this thread. Async callers must
/// use a current-thread runtime; spawned OS threads do not inherit the observer.
pub(crate) fn enter(capture: impl Capture) -> Guard {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        tracing::subscriber::set_global_default(Forward).expect("install test tracing dispatcher");
    });
    ACTIVE.with(|active| active.borrow_mut().push(Arc::new(capture)));
    Guard(PhantomData)
}

pub(crate) struct Guard(PhantomData<Rc<()>>);

impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            active.borrow_mut().pop();
        });
    }
}

fn forward(run: impl FnOnce(&dyn Capture)) {
    let capture = ACTIVE.with(|active| active.borrow().last().cloned());
    if let Some(capture) = capture {
        run(capture.as_ref());
    }
}

struct Forward;

impl tracing::Subscriber for Forward {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::always()
    }

    fn max_level_hint(&self) -> Option<tracing::metadata::LevelFilter> {
        Some(tracing::metadata::LevelFilter::TRACE)
    }

    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn event(&self, event: &tracing::Event<'_>) {
        forward(|capture| capture.event(event));
    }

    fn new_span(&self, attributes: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        static NEXT_SPAN: AtomicU64 = AtomicU64::new(1);
        forward(|capture| capture.new_span(attributes));
        tracing::span::Id::from_u64(NEXT_SPAN.fetch_add(1, Ordering::Relaxed))
    }

    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        forward(|capture| capture.record(values));
    }

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}
