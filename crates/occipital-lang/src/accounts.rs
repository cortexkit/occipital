//! Nested, host-owned resource accounts (§9.1). Reservations precede work;
//! dropping a reservation releases memory, never work already performed.

use std::sync::{Arc, Mutex, MutexGuard};

mod tree;

const MIB: u64 = 1024 * 1024;

/// Allocator rounding used by the retained-memory tariff, on every target.
pub const RETAINED_GRANULE: u64 = 16;
/// Conservative allocator metadata allowance, separate from Rust object headers.
pub const RETAINED_HEADER: u64 = 64;

/// One allocation's live payload and unused container capacity, in bytes.
/// Include Rust headers (for example Arc's two counters) in `requested`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetainedAllocation {
    /// Bytes the allocation holds, including Rust headers such as `Arc` counters.
    pub requested: u64,
    /// Capacity reserved but not used, such as a vector's spare slots.
    pub unused_capacity: u64,
}

impl RetainedAllocation {
    /// round_up(requested, 16) + 64 + unused capacity. Arithmetic overflow
    /// is a refusal, not a wrapped or saturating charge (§9.2).
    pub fn tariff(self) -> Option<u64> {
        self.requested
            .checked_add(RETAINED_GRANULE - 1)?
            .checked_div(RETAINED_GRANULE)?
            .checked_mul(RETAINED_GRANULE)?
            .checked_add(RETAINED_HEADER)?
            .checked_add(self.unused_capacity)
    }
}

/// The scope determines immutable language-version limits, not host policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountScope {
    /// One program instance's own aggregate (§9.1).
    Interface,
    /// One reply's own aggregate (§9.1).
    Reply,
    /// Every rich UI instance in one chat window, shared through a host handle.
    Chat,
    /// Every rich UI instance in the app, shared through a host handle.
    App,
}

/// Cumulative preparation work and currently retained bytes. Pending fuel and
/// expansion are included in `reserved`, so concurrent callers cannot overspend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccountUsage {
    /// Preparation fuel spent so far; it is never refilled (§9.1).
    pub spent_fuel: u64,
    /// Expanded bytes produced so far by preparation.
    pub expanded: u64,
    /// Bytes currently retained, released when their owner drops them.
    pub retained: u64,
    /// Fuel reserved by preparations still running.
    pub reserved_fuel: u64,
    /// Expanded bytes reserved by preparations still running.
    pub reserved_expanded: u64,
    /// How many preparations are running in this scope now.
    pub active_preparations: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Resources {
    pub fuel: u64,
    pub expanded: u64,
    pub retained: u64,
}

impl AccountScope {
    fn limits(self) -> Resources {
        match self {
            Self::Interface => Resources {
                fuel: 8_000_000,
                expanded: 32 * MIB,
                retained: 48 * MIB,
            },
            Self::Reply => Resources {
                fuel: 6_000_000,
                expanded: 16 * MIB,
                retained: 24 * MIB,
            },
            Self::Chat => Resources {
                fuel: 24_000_000,
                expanded: 64 * MIB,
                retained: 96 * MIB,
            },
            // App preparation is governed by chat queues, not a second fuel cap.
            Self::App => Resources {
                fuel: u64::MAX,
                expanded: u64::MAX,
                retained: 128 * MIB,
            },
        }
    }
}

/// A host keeps one chat handle per opening/measurement window and one app
/// handle for all concurrent rich UI. Clone handles, not balances. No reset API
/// exists: eviction and program Reset cannot replenish cumulative work (§9.1).
#[derive(Clone, Debug)]
pub struct SharedAccount {
    scope: AccountScope,
    usage: Arc<Mutex<AccountUsage>>,
}

impl SharedAccount {
    /// A fresh chat-window account. Create one per chat window and clone it.
    pub fn chat() -> Self {
        Self::new(AccountScope::Chat)
    }
    /// A fresh app-wide account. Create one per app process and clone it.
    pub fn app() -> Self {
        Self::new(AccountScope::App)
    }

    fn new(scope: AccountScope) -> Self {
        Self {
            scope,
            usage: Arc::new(Mutex::new(AccountUsage::default())),
        }
    }

    fn lock(&self) -> MutexGuard<'_, AccountUsage> {
        self.usage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The scope this handle charges.
    pub fn scope(&self) -> AccountScope {
        self.scope
    }
    /// A snapshot of this account's usage.
    pub fn usage(&self) -> AccountUsage {
        *self.lock()
    }

    /// Reserve host-reported memory in this scope before retaining the buffer.
    /// An instance also reserves its local and other shared scope; this method
    /// supports external host work recorded in deterministic replay inputs.
    pub fn report_allocation(
        &self,
        allocation: RetainedAllocation,
    ) -> Result<SharedRetention, AccountExhausted> {
        let bytes = allocation
            .tariff()
            .ok_or(AccountExhausted { scope: self.scope })?;
        let mut usage = self.lock();
        if usage
            .retained
            .checked_add(bytes)
            .is_none_or(|n| n > self.scope.limits().retained)
        {
            return Err(AccountExhausted { scope: self.scope });
        }
        usage.retained += bytes;
        Ok(SharedRetention {
            account: self.clone(),
            bytes,
        })
    }
}

/// Keep this lease for as long as the reported allocation is retained. Share a
/// lease with Arc when several host caches own the same buffer, not by copying it.
#[derive(Debug)]
pub struct SharedRetention {
    account: SharedAccount,
    bytes: u64,
}

impl Drop for SharedRetention {
    fn drop(&mut self) {
        self.account.lock().retained -= self.bytes;
    }
}

/// The account that refused admission. Hosts queue or use a fixed fallback;
/// sites expose `too_large` rather than continuing uncharged preparation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountExhausted {
    /// The scope whose budget is exhausted.
    pub scope: AccountScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A shared account handle passed for the wrong scope. Each variant holds the
/// scope the handle actually has.
pub enum ScopeError {
    /// An instance's own scope must be `Interface` or `Reply`.
    Local(AccountScope),
    /// The chat handle is not a chat account.
    Chat(AccountScope),
    /// The app handle is not an app account.
    App(AccountScope),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A preparation tried to finish with more than it reserved.
pub enum ReservationError {
    /// Retained bytes grew after the reservation was made.
    RetainedIncrease,
    /// The work spent more fuel than it reserved.
    FuelExceedsReserved,
    /// The work expanded more bytes than it reserved.
    ExpansionExceedsReserved,
}

/// Each instance owns its local scope and shares the host's chat/app handles.
#[derive(Clone, Debug)]
pub(crate) struct Accounts {
    local: SharedAccount,
    chat: SharedAccount,
    app: SharedAccount,
    abort: Arc<Mutex<Option<crate::eval::Abort>>>,
}

impl Accounts {
    pub fn reserve_parse(&self, prefix_bytes: usize) -> Result<ParseReservation, Refused> {
        let fuel = (prefix_bytes as u64)
            .checked_mul(2)
            .filter(|n| *n <= UnitKind::Parse.budget())
            .ok_or(Refused::UnitFuel)?;
        let reservation = if self.local.scope == AccountScope::Reply {
            Some(
                self.reserve(Resources {
                    fuel,
                    ..Resources::default()
                })
                .map_err(Refused::Aggregate)?,
            )
        } else {
            None
        };
        Ok(ParseReservation { reservation })
    }
    pub fn chat_handle(&self) -> SharedAccount {
        self.chat.clone()
    }
    pub fn app_handle(&self) -> SharedAccount {
        self.app.clone()
    }
    pub fn new(
        scope: AccountScope,
        chat: SharedAccount,
        app: SharedAccount,
    ) -> Result<Self, ScopeError> {
        if !matches!(scope, AccountScope::Interface | AccountScope::Reply) {
            return Err(ScopeError::Local(scope));
        }
        if chat.scope != AccountScope::Chat {
            return Err(ScopeError::Chat(chat.scope));
        }
        if app.scope != AccountScope::App {
            return Err(ScopeError::App(app.scope));
        }
        Ok(Self::validated(scope, chat, app))
    }

    fn validated(scope: AccountScope, chat: SharedAccount, app: SharedAccount) -> Self {
        Self {
            local: SharedAccount::new(scope),
            chat,
            app,
            abort: Arc::new(Mutex::new(None)),
        }
    }

    pub fn isolated(scope: AccountScope) -> Result<Self, ScopeError> {
        Self::new(scope, SharedAccount::chat(), SharedAccount::app())
    }

    pub fn interface() -> Self {
        Self::validated(
            AccountScope::Interface,
            SharedAccount::chat(),
            SharedAccount::app(),
        )
    }

    pub fn reply() -> Self {
        Self::validated(
            AccountScope::Reply,
            SharedAccount::chat(),
            SharedAccount::app(),
        )
    }

    pub fn fresh_local(&self) -> Self {
        Self::validated(self.local.scope, self.chat_handle(), self.app_handle())
    }

    fn handles(&self) -> [&SharedAccount; 3] {
        [&self.local, &self.chat, &self.app]
    }

    pub fn take_abort(&self) -> Option<crate::eval::Abort> {
        self.abort
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// Lock in scope order everywhere, and hold all locks through admission.
    /// No observer can see a partial nested reservation, even across threads.
    pub fn reserve(&self, resources: Resources) -> Result<Reservation, AccountExhausted> {
        let handles = self.handles();
        let mut guards = handles.map(SharedAccount::lock);
        for (account, usage) in handles.iter().zip(&guards) {
            let limit = account.scope.limits();
            let fits = |spent: u64, pending: u64, add: u64, cap: u64| {
                spent
                    .checked_add(pending)
                    .and_then(|n| n.checked_add(add))
                    .is_some_and(|n| n <= cap)
            };
            if !fits(
                usage.spent_fuel,
                usage.reserved_fuel,
                resources.fuel,
                limit.fuel,
            ) || !fits(
                usage.expanded,
                usage.reserved_expanded,
                resources.expanded,
                limit.expanded,
            ) || !fits(usage.retained, 0, resources.retained, limit.retained)
            {
                if account.scope == AccountScope::Interface {
                    let fuel_failure = !fits(
                        usage.spent_fuel,
                        usage.reserved_fuel,
                        resources.fuel,
                        limit.fuel,
                    );
                    *self
                        .abort
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some(if fuel_failure {
                            crate::eval::Abort::Fuel
                        } else {
                            crate::eval::Abort::LimitAlloc
                        });
                }
                return Err(AccountExhausted {
                    scope: account.scope,
                });
            }
        }
        for usage in &mut guards {
            usage.reserved_fuel += resources.fuel;
            usage.reserved_expanded += resources.expanded;
            usage.retained += resources.retained;
        }
        Ok(Reservation {
            accounts: self.clone(),
            pending: resources,
        })
    }

    pub fn retain(&self, allocation: RetainedAllocation) -> Result<Reservation, AccountExhausted> {
        let retained = allocation.tariff().ok_or(AccountExhausted {
            scope: self.local.scope,
        })?;
        self.reserve(Resources {
            retained,
            ..Resources::default()
        })
    }

    pub fn begin(&self, kind: UnitKind) -> Result<Unit, AccountExhausted> {
        if kind.preparation()
            || (kind == UnitKind::Parse && self.local.scope == AccountScope::Reply)
        {
            // A zero-byte source must not start an otherwise exhausted account.
            // Reserve one fuel before even entering the preparation callback.
            let probe = self.reserve(Resources {
                fuel: 1,
                expanded: 1,
                retained: 1,
            })?;
            drop(probe);
        }
        let slot = if kind.preparation() {
            let mut usage = self.chat.lock();
            if usage.active_preparations == 2 {
                return Err(AccountExhausted {
                    scope: AccountScope::Chat,
                });
            }
            usage.active_preparations += 1;
            Some(PreparationSlot(self.chat.clone()))
        } else {
            None
        };
        Ok(Unit {
            accounts: self.clone(),
            kind,
            fuel: 0,
            expanded: 0,
            retained: Vec::new(),
            metadata: None,
            _slot: slot,
        })
    }
}

pub(crate) struct ParseReservation {
    reservation: Option<Reservation>,
}

impl ParseReservation {
    pub fn settle(mut self, parsed_bytes: usize) -> Result<(), Refused> {
        if let Some(reservation) = &mut self.reservation {
            return reservation.settle_parse(parsed_bytes);
        }
        let fuel = parsed_fuel(parsed_bytes)?;
        if fuel > UnitKind::Parse.budget() {
            return Err(Refused::UnitFuel);
        }
        Ok(())
    }
}

fn parsed_fuel(parsed_bytes: usize) -> Result<u64, Refused> {
    (parsed_bytes as u64)
        .checked_mul(2)
        .ok_or(Refused::ArithmeticOverflow)
}

/// A reservation owns only pending work and retained memory. Cloned owners
/// should share it with Arc; duplicating the token would release bytes twice.
#[derive(Debug)]
pub(crate) struct Reservation {
    accounts: Accounts,
    pending: Resources,
}

impl Reservation {
    fn resize_retained(&mut self, retained: u64) -> Result<(), AccountExhausted> {
        if retained <= self.pending.retained {
            // The branch proves shrink's direction without a fallible assertion.
            self.release_retained(retained);
            return Ok(());
        }
        let mut extra = self.accounts.reserve(Resources {
            retained: retained - self.pending.retained,
            ..Resources::default()
        })?;
        extra.pending.retained = 0;
        self.pending.retained = retained;
        Ok(())
    }
    pub fn shrink_retained(&mut self, retained: u64) -> Result<(), ReservationError> {
        if retained > self.pending.retained {
            return Err(ReservationError::RetainedIncrease);
        }
        self.release_retained(retained);
        Ok(())
    }
    fn release_retained(&mut self, retained: u64) {
        let released = self.pending.retained - retained;
        for account in self.accounts.handles() {
            account.lock().retained -= released;
        }
        self.pending.retained = retained;
    }
    pub fn spend(&mut self, fuel: u64, expanded: u64) -> Result<(), ReservationError> {
        if fuel > self.pending.fuel {
            return Err(ReservationError::FuelExceedsReserved);
        }
        if expanded > self.pending.expanded {
            return Err(ReservationError::ExpansionExceedsReserved);
        }
        for account in self.accounts.handles() {
            let mut usage = account.lock();
            usage.reserved_fuel -= fuel;
            usage.spent_fuel += fuel;
            usage.reserved_expanded -= expanded;
            usage.expanded += expanded;
        }
        self.pending.fuel -= fuel;
        self.pending.expanded -= expanded;
        Ok(())
    }

    fn settle_fuel(&mut self, fuel: u64) -> Result<(), Refused> {
        // Work has already happened: even a refused excess cannot refund the
        // original reservation. A successful settlement charges every byte.
        let prepaid = fuel.min(self.pending.fuel);
        self.spend(prepaid, 0).map_err(Refused::Reservation)?;
        let excess = fuel - prepaid;
        if excess != 0 {
            let mut extra = self
                .accounts
                .reserve(Resources {
                    fuel: excess,
                    ..Resources::default()
                })
                .map_err(Refused::Aggregate)?;
            extra.spend(excess, 0).map_err(Refused::Reservation)?;
        }
        Ok(())
    }

    fn settle_parse(&mut self, parsed_bytes: usize) -> Result<(), Refused> {
        let fuel = match parsed_fuel(parsed_bytes) {
            Ok(fuel) => fuel,
            Err(error) => {
                self.spend(self.pending.fuel, 0)
                    .map_err(Refused::Reservation)?;
                return Err(error);
            }
        };
        self.settle_fuel(fuel)?;
        if fuel > UnitKind::Parse.budget() {
            return Err(Refused::UnitFuel);
        }
        Ok(())
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        for account in self.accounts.handles() {
            let mut usage = account.lock();
            usage.reserved_fuel -= self.pending.fuel;
            usage.reserved_expanded -= self.pending.expanded;
            usage.retained -= self.pending.retained;
        }
    }
}

/// The five work units have independent fuel, so fallback survives exhaustion
/// of evaluation or preparation. Reply provisional work has a separate cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnitKind {
    Evaluation,
    Parse,
    DataPreparation,
    AnswerConversion,
    Fallback,
}

impl UnitKind {
    pub fn budget(self) -> u64 {
        match self {
            Self::Evaluation => 1_000_000,
            Self::Parse => 600_000,
            Self::DataPreparation | Self::AnswerConversion => 3_000_000,
            Self::Fallback => 400_000,
        }
    }
    fn preparation(self) -> bool {
        matches!(self, Self::DataPreparation | Self::AnswerConversion)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Why an account refused work before it started (§9.1). Work is never
/// continued uncharged after a refusal.
pub enum Refused {
    /// The syntax mode has no account rules (only reply and program modes do).
    UnsupportedMode,
    /// The work's own fuel or size limit would be exceeded.
    UnitFuel,
    /// An aggregate account (instance, reply, chat or app) is exhausted.
    Aggregate(AccountExhausted),
    /// A shared account handle has the wrong scope.
    Scope(ScopeError),
    /// A finished preparation exceeded its reservation.
    Reservation(ReservationError),
    /// A charge overflowed; it is refused instead of wrapping.
    ArithmeticOverflow,
    /// A retained charge was released twice or never made.
    MissingRetention,
}

#[derive(Debug)]
pub(crate) struct Unit {
    accounts: Accounts,
    kind: UnitKind,
    pub fuel: u64,
    pub expanded: u64,
    retained: Vec<Reservation>,
    metadata: Option<Reservation>,
    _slot: Option<PreparationSlot>,
}

/// Charges include the ownership ledger itself, not only the data it owns.
#[derive(Debug)]
pub(crate) struct Retention {
    _allocations: Vec<Reservation>,
    _metadata: Reservation,
}

#[derive(Debug)]
struct PreparationSlot(SharedAccount);

impl Drop for PreparationSlot {
    fn drop(&mut self) {
        self.0.lock().active_preparations -= 1;
    }
}

impl Unit {
    pub fn shrink_last(&mut self, allocation: RetainedAllocation) -> Result<(), Refused> {
        let retained = allocation.tariff().ok_or(Refused::ArithmeticOverflow)?;
        self.retained
            .last_mut()
            .ok_or(Refused::MissingRetention)?
            .shrink_retained(retained)
            .map_err(Refused::Reservation)
    }
    pub fn charge(&mut self, fuel: u64) -> Result<(), Refused> {
        let total = self
            .fuel
            .checked_add(fuel)
            .filter(|n| *n <= self.kind.budget())
            .ok_or(Refused::UnitFuel)?;
        if self.kind.preparation()
            || (self.kind == UnitKind::Parse && self.accounts.local.scope == AccountScope::Reply)
        {
            let mut reservation = self
                .accounts
                .reserve(Resources {
                    fuel,
                    ..Resources::default()
                })
                .map_err(Refused::Aggregate)?;
            reservation.spend(fuel, 0).map_err(Refused::Reservation)?;
        }
        self.fuel = total;
        Ok(())
    }

    pub fn expand(&mut self, bytes: u64) -> Result<(), Refused> {
        let total = self.expanded.checked_add(bytes).ok_or(Refused::UnitFuel)?;
        let mut reservation = self
            .accounts
            .reserve(Resources {
                expanded: bytes,
                ..Resources::default()
            })
            .map_err(Refused::Aggregate)?;
        reservation.spend(0, bytes).map_err(Refused::Reservation)?;
        self.expanded = total;
        Ok(())
    }

    pub fn retain(&mut self, allocation: RetainedAllocation) -> Result<(), Refused> {
        let lease = self
            .accounts
            .retain(allocation)
            .map_err(Refused::Aggregate)?;
        let len = self
            .retained
            .len()
            .checked_add(1)
            .ok_or(Refused::UnitFuel)?;
        let capacity = if len > self.retained.capacity() {
            self.retained
                .capacity()
                .checked_mul(2)
                .ok_or(Refused::UnitFuel)?
                .max(4)
        } else {
            self.retained.capacity()
        };
        {
            let slot = std::mem::size_of::<Reservation>() as u64;
            let backing = RetainedAllocation {
                requested: (len as u64).checked_mul(slot).ok_or(Refused::UnitFuel)?,
                unused_capacity: ((capacity - len) as u64)
                    .checked_mul(slot)
                    .ok_or(Refused::UnitFuel)?,
            }
            .tariff()
            .ok_or(Refused::UnitFuel)?;
            let header = RetainedAllocation {
                requested: (2 * std::mem::size_of::<usize>() + std::mem::size_of::<Retention>())
                    as u64,
                unused_capacity: 0,
            }
            .tariff()
            .ok_or(Refused::UnitFuel)?;
            let bytes = backing.checked_add(header).ok_or(Refused::UnitFuel)?;
            if let Some(metadata) = &mut self.metadata {
                metadata
                    .resize_retained(bytes)
                    .map_err(Refused::Aggregate)?;
            } else {
                self.metadata = Some(
                    self.accounts
                        .reserve(Resources {
                            retained: bytes,
                            ..Resources::default()
                        })
                        .map_err(Refused::Aggregate)?,
                );
            }
            if len > self.retained.capacity() {
                self.retained.reserve_exact(capacity - self.retained.len());
            }
        }
        self.retained.push(lease);
        Ok(())
    }

    pub fn finish(self) -> Result<Arc<Retention>, Refused> {
        let metadata = self.metadata.ok_or(Refused::MissingRetention)?;
        Ok(Arc::new(Retention {
            _allocations: self.retained,
            _metadata: metadata,
        }))
    }
}

/// A streamed reply protects its final bounded parse from provisional work
/// (§3.10). The reply session supplies the actual parser/measurement callbacks.
pub(crate) struct ReplyWork {
    accounts: Accounts,
    final_parse: Reservation,
    provisional: u64,
}

/// Admission of an already computed per-revision estimate is not retention.
/// Geometry supplies the context-specific estimate; host allocation reports
/// must still reserve their actual memory independently (§9.1).
#[derive(Debug)]
pub(crate) struct RevisionReservation {
    pub visits: u64,
    pub decoded: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RevisionLimit {
    Visits,
    DecodedBytes,
}

impl RevisionReservation {
    pub fn new(visits: u64, decoded: u64) -> Result<Self, RevisionLimit> {
        if visits > 100_000 {
            return Err(RevisionLimit::Visits);
        }
        if decoded > 32 * MIB {
            return Err(RevisionLimit::DecodedBytes);
        }
        Ok(Self { visits, decoded })
    }
}

impl ReplyWork {
    pub fn new(accounts: &Accounts) -> Result<Self, Refused> {
        if accounts.local.scope != AccountScope::Reply {
            return Err(Refused::Scope(ScopeError::Local(accounts.local.scope)));
        }
        let final_parse = accounts
            .reserve(Resources {
                fuel: 600_000,
                ..Resources::default()
            })
            .map_err(Refused::Aggregate)?;
        Ok(Self {
            accounts: accounts.clone(),
            final_parse,
            provisional: 0,
        })
    }

    pub fn provisional(&mut self, fuel: u64) -> Result<(), Refused> {
        let total = self
            .provisional
            .checked_add(fuel)
            .filter(|n| *n <= 1_400_000)
            .ok_or(Refused::UnitFuel)?;
        let mut reservation = self
            .accounts
            .reserve(Resources {
                fuel,
                ..Resources::default()
            })
            .map_err(Refused::Aggregate)?;
        reservation.spend(fuel, 0).map_err(Refused::Reservation)?;
        self.provisional = total;
        Ok(())
    }

    pub fn finish<T>(mut self, parse: impl FnOnce() -> (T, usize)) -> Result<T, Refused> {
        let (result, parsed_bytes) = parse();
        self.final_parse.settle_parse(parsed_bytes)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
