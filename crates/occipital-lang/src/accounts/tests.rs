use super::*;

#[test]
fn scope_mismatches_are_typed_errors() {
    for scope in [AccountScope::Chat, AccountScope::App] {
        assert_eq!(
            Accounts::new(scope, SharedAccount::chat(), SharedAccount::app()).unwrap_err(),
            ScopeError::Local(scope)
        );
    }
    assert_eq!(
        Accounts::new(
            AccountScope::Interface,
            SharedAccount::app(),
            SharedAccount::app()
        )
        .unwrap_err(),
        ScopeError::Chat(AccountScope::App)
    );
    assert_eq!(
        Accounts::new(
            AccountScope::Reply,
            SharedAccount::chat(),
            SharedAccount::chat()
        )
        .unwrap_err(),
        ScopeError::App(AccountScope::Chat)
    );
    let accounts = Accounts::interface();
    assert_eq!(
        ReplyWork::new(&accounts).err(),
        Some(Refused::Scope(ScopeError::Local(AccountScope::Interface)))
    );
    assert_eq!(accounts.chat.usage(), AccountUsage::default());
}

#[test]
fn invalid_reservation_and_ownership_operations_are_atomic_typed_errors() {
    let accounts = Accounts::interface();
    let mut lease = accounts.reserve(resources(10, 20, 30)).unwrap();
    let before = accounts.chat.usage();
    assert_eq!(
        lease.spend(11, 0),
        Err(ReservationError::FuelExceedsReserved)
    );
    assert_eq!(
        lease.spend(1, 21),
        Err(ReservationError::ExpansionExceedsReserved)
    );
    assert_eq!(
        lease.shrink_retained(31),
        Err(ReservationError::RetainedIncrease)
    );
    assert_eq!(accounts.chat.usage(), before);
    lease.spend(10, 20).unwrap();
    lease.shrink_retained(0).unwrap();
    drop(lease);
    assert_eq!(accounts.chat.usage().spent_fuel, 10);
    assert_eq!(accounts.chat.usage().expanded, 20);
    assert_eq!(accounts.chat.usage().retained, 0);

    let mut unit = accounts.begin(UnitKind::Evaluation).unwrap();
    assert_eq!(
        unit.shrink_last(RetainedAllocation {
            requested: 0,
            unused_capacity: 0
        }),
        Err(Refused::MissingRetention)
    );
    assert!(matches!(unit.finish(), Err(Refused::MissingRetention)));
    let mut unit = accounts.begin(UnitKind::Evaluation).unwrap();
    unit.retain(RetainedAllocation {
        requested: 8,
        unused_capacity: 0,
    })
    .unwrap();
    let before = accounts.chat.usage();
    assert_eq!(
        unit.shrink_last(RetainedAllocation {
            requested: u64::MAX,
            unused_capacity: 0
        }),
        Err(Refused::ArithmeticOverflow)
    );
    assert_eq!(
        unit.shrink_last(RetainedAllocation {
            requested: 256,
            unused_capacity: 0
        }),
        Err(Refused::Reservation(ReservationError::RetainedIncrease))
    );
    assert_eq!(accounts.chat.usage(), before);
    unit.shrink_last(RetainedAllocation {
        requested: 0,
        unused_capacity: 0,
    })
    .unwrap();
    let retained = unit.finish().unwrap();
    assert_eq!(accounts.chat.usage().retained, before.retained - 16);
    drop(retained);
    assert_eq!(accounts.chat.usage().retained, 0);
}

fn spend_chat_except(chat: &SharedAccount, app: &SharedAccount, remaining: u64) {
    let mut fuel = 24_000_000 - remaining;
    while fuel != 0 {
        let amount = fuel.min(8_000_000);
        let accounts = Accounts::new(AccountScope::Interface, chat.clone(), app.clone()).unwrap();
        let mut reservation = accounts.reserve(resources(amount, 0, 0)).unwrap();
        reservation.spend(amount, 0).unwrap();
        fuel -= amount;
    }
}

#[test]
fn parse_and_reply_settlement_charge_excess_or_return_typed_refusal() {
    let accounts = Accounts::reply();
    let reservation = accounts.reserve_parse(10).unwrap();
    assert_eq!(accounts.chat.usage().reserved_fuel, 20);
    reservation.settle(12).unwrap();
    assert_eq!(accounts.chat.usage().spent_fuel, 24);
    assert_eq!(accounts.chat.usage().reserved_fuel, 0);

    let chat = SharedAccount::chat();
    let app = SharedAccount::app();
    spend_chat_except(&chat, &app, 20);
    let accounts = Accounts::new(AccountScope::Reply, chat.clone(), app).unwrap();
    assert_eq!(
        accounts.reserve_parse(10).unwrap().settle(12),
        Err(Refused::Aggregate(AccountExhausted {
            scope: AccountScope::Chat
        }))
    );
    assert_eq!(chat.usage().spent_fuel, 24_000_000);
    assert_eq!(chat.usage().reserved_fuel, 0);
    assert_eq!(accounts.local.usage().spent_fuel, 20);

    let accounts = Accounts::reply();
    assert_eq!(
        ReplyWork::new(&accounts).unwrap().finish(|| ((), 300_001)),
        Err(Refused::UnitFuel)
    );
    assert_eq!(accounts.chat.usage().spent_fuel, 600_002);
    assert_eq!(accounts.chat.usage().reserved_fuel, 0);

    let chat = SharedAccount::chat();
    let app = SharedAccount::app();
    spend_chat_except(&chat, &app, 600_000);
    let accounts = Accounts::new(AccountScope::Reply, chat.clone(), app).unwrap();
    assert_eq!(
        ReplyWork::new(&accounts).unwrap().finish(|| ((), 300_001)),
        Err(Refused::Aggregate(AccountExhausted {
            scope: AccountScope::Chat
        }))
    );
    assert_eq!(chat.usage().spent_fuel, 24_000_000);
    assert_eq!(chat.usage().reserved_fuel, 0);

    let accounts = Accounts::reply();
    assert_eq!(
        accounts.reserve_parse(1).unwrap().settle(300_001),
        Err(Refused::UnitFuel)
    );
    assert_eq!(accounts.chat.usage().spent_fuel, 600_002);
    let accounts = Accounts::interface();
    assert_eq!(
        accounts.reserve_parse(1).unwrap().settle(300_001),
        Err(Refused::UnitFuel)
    );

    let accounts = Accounts::reply();
    assert_eq!(
        accounts.reserve_parse(1).unwrap().settle(usize::MAX),
        Err(Refused::ArithmeticOverflow)
    );
    assert_eq!(accounts.chat.usage().spent_fuel, 2);
    assert_eq!(accounts.chat.usage().reserved_fuel, 0);
    let accounts = Accounts::reply();
    assert_eq!(
        ReplyWork::new(&accounts)
            .unwrap()
            .finish(|| ((), usize::MAX)),
        Err(Refused::ArithmeticOverflow)
    );
    assert_eq!(accounts.chat.usage().spent_fuel, 600_000);
    assert_eq!(accounts.chat.usage().reserved_fuel, 0);
}

#[test]
fn revision_reservation_reports_the_named_limit() {
    assert_eq!(
        RevisionReservation::new(100_001, 0).unwrap_err(),
        RevisionLimit::Visits
    );
    assert_eq!(
        RevisionReservation::new(0, 32 * MIB + 1).unwrap_err(),
        RevisionLimit::DecodedBytes
    );
}

fn resources(fuel: u64, expanded: u64, retained: u64) -> Resources {
    Resources {
        fuel,
        expanded,
        retained,
    }
}

#[test]
fn five_unit_fuel_bounds_are_exact_and_fallback_independent() {
    for (kind, limit) in [
        (UnitKind::Evaluation, 1_000_000),
        (UnitKind::Parse, 600_000),
        (UnitKind::DataPreparation, 3_000_000),
        (UnitKind::AnswerConversion, 3_000_000),
        (UnitKind::Fallback, 400_000),
    ] {
        let accounts = Accounts::isolated(AccountScope::Interface).unwrap();
        let mut unit = accounts.begin(kind).unwrap();
        unit.charge(limit).unwrap();
        assert_eq!(unit.fuel, limit);
        assert_eq!(unit.charge(1), Err(Refused::UnitFuel));
        assert_eq!(unit.fuel, limit);
        let mut fallback = accounts.begin(UnitKind::Fallback).unwrap();
        fallback.charge(400_000).unwrap();
    }
}

#[test]
fn interface_and_reply_limits_are_exact_and_refusals_are_atomic() {
    for (scope, fuel, expanded, retained) in [
        (AccountScope::Interface, 8_000_000, 32 * MIB, 48 * MIB),
        (AccountScope::Reply, 6_000_000, 16 * MIB, 24 * MIB),
    ] {
        let accounts = Accounts::isolated(scope).unwrap();
        let mut reservation = accounts
            .reserve(resources(fuel, expanded, retained))
            .unwrap();
        for excess in [resources(1, 0, 0), resources(0, 1, 0), resources(0, 0, 1)] {
            assert_eq!(
                accounts.reserve(excess).unwrap_err(),
                AccountExhausted { scope }
            );
        }
        let before = accounts.chat.usage();
        assert_eq!(before.reserved_fuel, fuel);
        assert_eq!(before.reserved_expanded, expanded);
        assert_eq!(before.retained, retained);
        reservation.spend(fuel, expanded).unwrap();
        drop(reservation);
        assert_eq!(
            accounts.chat.usage(),
            AccountUsage {
                spent_fuel: fuel,
                expanded,
                ..AccountUsage::default()
            }
        );
        assert!(accounts.reserve(resources(1, 0, 1)).is_err());
        assert_eq!(accounts.chat.usage().retained, 0);
    }
}

#[test]
fn two_instances_refuse_exhausted_chat_before_preparation_and_reload_does_not_refill() {
    let chat = SharedAccount::chat();
    let app = SharedAccount::app();
    for _ in 0..3 {
        let accounts = Accounts::new(AccountScope::Interface, chat.clone(), app.clone()).unwrap();
        let mut reservation = accounts.reserve(resources(8_000_000, 0, 0)).unwrap();
        reservation.spend(8_000_000, 0).unwrap();
    }
    let mut work = 0;
    for _ in 0..2 {
        let accounts = Accounts::new(AccountScope::Interface, chat.clone(), app.clone()).unwrap();
        let result = accounts.begin(UnitKind::DataPreparation).map(|_| work += 1);
        assert_eq!(result.unwrap_err().scope, AccountScope::Chat);
    }
    assert_eq!(work, 0);
    assert_eq!(chat.usage().spent_fuel, 24_000_000);
}

#[test]
fn chat_expansion_and_retention_cover_all_instances() {
    let chat = SharedAccount::chat();
    let app = SharedAccount::app();
    let mut leases = Vec::new();
    for _ in 0..2 {
        let accounts = Accounts::new(AccountScope::Interface, chat.clone(), app.clone()).unwrap();
        let mut lease = accounts.reserve(resources(0, 32 * MIB, 48 * MIB)).unwrap();
        lease.spend(0, 32 * MIB).unwrap();
        leases.push(lease);
    }
    let third = Accounts::new(AccountScope::Interface, chat.clone(), app).unwrap();
    assert_eq!(
        third.reserve(resources(0, 1, 0)).unwrap_err().scope,
        AccountScope::Chat
    );
    assert_eq!(
        third.reserve(resources(0, 0, 1)).unwrap_err().scope,
        AccountScope::Chat
    );
    drop(leases);
    assert_eq!(chat.usage().expanded, 64 * MIB);
    assert_eq!(chat.usage().retained, 0);
}

#[test]
fn app_retention_spans_chat_windows_and_releases_on_eviction() {
    let app = SharedAccount::app();
    let mut leases = Vec::new();
    for bytes in [48 * MIB, 48 * MIB, 32 * MIB] {
        let accounts =
            Accounts::new(AccountScope::Interface, SharedAccount::chat(), app.clone()).unwrap();
        leases.push(accounts.reserve(resources(0, 0, bytes)).unwrap());
    }
    let accounts = Accounts::new(AccountScope::Reply, SharedAccount::chat(), app.clone()).unwrap();
    assert_eq!(
        accounts.reserve(resources(0, 0, 1)).unwrap_err().scope,
        AccountScope::App
    );
    assert_eq!(app.usage().retained, 128 * MIB);
    drop(leases.pop());
    assert!(accounts.reserve(resources(0, 0, 24 * MIB)).is_ok());
}

#[test]
fn cancellation_releases_pending_but_not_spent_work() {
    let accounts = Accounts::isolated(AccountScope::Interface).unwrap();
    let mut pending = accounts.reserve(resources(300, 400, 500)).unwrap();
    pending.spend(100, 200).unwrap();
    drop(pending);
    for account in accounts.handles() {
        assert_eq!(
            account.usage(),
            AccountUsage {
                spent_fuel: 100,
                expanded: 200,
                ..AccountUsage::default()
            }
        );
    }
}

#[test]
fn at_most_two_preparations_per_chat_release_slots_on_cancel() {
    let chat = SharedAccount::chat();
    let app = SharedAccount::app();
    let first = Accounts::new(AccountScope::Interface, chat.clone(), app.clone()).unwrap();
    let second = Accounts::new(AccountScope::Reply, chat.clone(), app).unwrap();
    let one = first.begin(UnitKind::DataPreparation).unwrap();
    let two = second.begin(UnitKind::AnswerConversion).unwrap();
    assert_eq!(chat.usage().active_preparations, 2);
    assert_eq!(
        first.begin(UnitKind::DataPreparation).unwrap_err().scope,
        AccountScope::Chat
    );
    drop(one);
    let three = first.begin(UnitKind::DataPreparation).unwrap();
    drop((two, three));
    assert_eq!(chat.usage().active_preparations, 0);
}

#[test]
fn reply_parse_charges_local_and_chat_but_program_parse_does_not_spend_preparation() {
    for scope in [AccountScope::Interface, AccountScope::Reply] {
        let accounts = Accounts::isolated(scope).unwrap();
        accounts
            .begin(UnitKind::Parse)
            .unwrap()
            .charge(600_000)
            .unwrap();
        assert_eq!(
            accounts.chat.usage().spent_fuel,
            if scope == AccountScope::Reply {
                600_000
            } else {
                0
            }
        );
    }
}

#[test]
fn host_buffers_charge_retained_separately_from_decode_estimates() {
    let accounts = Accounts::isolated(AccountScope::Reply).unwrap();
    let estimate = RevisionReservation::new(100_000, 32 * MIB).unwrap();
    assert_eq!(estimate.visits, 100_000);
    assert_eq!(estimate.decoded, 33_554_432);
    assert!(RevisionReservation::new(100_000, 32 * MIB + 1).is_err());
    assert!(RevisionReservation::new(100_001, 0).is_err());
    assert_eq!(accounts.app.usage().retained, 0);
    assert!(accounts
        .retain(RetainedAllocation {
            requested: estimate.decoded,
            unused_capacity: 0
        })
        .is_err());
    let raw = accounts
        .retain(RetainedAllocation {
            requested: 100,
            unused_capacity: 0,
        })
        .unwrap();
    let decoded = accounts
        .retain(RetainedAllocation {
            requested: 4096,
            unused_capacity: 0,
        })
        .unwrap();
    let measurement = accounts
        .retain(RetainedAllocation {
            requested: 80,
            unused_capacity: 48,
        })
        .unwrap();
    for account in accounts.handles() {
        assert_eq!(account.usage().retained, 176 + 4160 + 192);
        assert_eq!(account.usage().expanded, 0);
    }
    drop((raw, decoded, measurement));
    assert_eq!(accounts.app.usage().retained, 0);
}

#[test]
fn retained_tariff_has_fixed_header_rounding_capacity_and_checked_arithmetic() {
    for (requested, unused, expected) in [(0, 0, 64), (1, 0, 80), (16, 0, 80), (17, 9, 105)] {
        assert_eq!(
            RetainedAllocation {
                requested,
                unused_capacity: unused
            }
            .tariff(),
            Some(expected)
        );
    }
    assert_eq!(
        RetainedAllocation {
            requested: u64::MAX,
            unused_capacity: 0
        }
        .tariff(),
        None
    );
    assert_eq!(
        RetainedAllocation {
            requested: 1,
            unused_capacity: u64::MAX
        }
        .tariff(),
        None
    );
}

#[test]
fn counting_allocator_requested_bytes_fit_each_retained_tariff() {
    for len in [1usize, 15, 16, 17, 255, 4097] {
        let capacity = len + 37;
        let mut buffer = None;
        let measured = allocation_counter::measure(|| {
            buffer = Some(std::hint::black_box(Vec::<u8>::with_capacity(capacity)));
        });
        let tariff = RetainedAllocation {
            requested: len as u64,
            unused_capacity: 37,
        }
        .tariff()
        .unwrap();
        assert_eq!(measured.count_total, 1);
        assert_eq!(measured.bytes_total, capacity as u64);
        assert!(measured.bytes_total <= tariff);
        drop(buffer);

        let source = "x".repeat(len);
        let mut shared = None;
        let measured = allocation_counter::measure(|| {
            shared = Some(std::hint::black_box(Arc::<str>::from(source.as_str())));
        });
        let arc_tariff = RetainedAllocation {
            requested: len as u64 + 16,
            unused_capacity: 0,
        }
        .tariff()
        .unwrap();
        assert_eq!(measured.count_total, 1);
        assert!(measured.bytes_total <= arc_tariff);
        drop(shared);
    }
}

#[test]
fn prepared_descendants_keep_memory_charged_but_eviction_does_not_refill_expansion() {
    let accounts = Accounts::isolated(AccountScope::Reply).unwrap();
    let prepared = crate::data::prepare_accounted(
        &accounts,
        UnitKind::DataPreparation,
        br#"{"a":["hello",7]}"#,
        crate::data::Format::Json,
        &Default::default(),
        crate::SourceSpan {
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 1,
        },
    )
    .unwrap();
    let crate::ValueKind::Record(record) = prepared.value.kind() else {
        panic!()
    };
    let child = record.get("a");
    let before = accounts.chat.usage();
    assert!(before.retained > 0);
    assert_eq!(before.expanded, 16 + 20 + 16 + 36 + 16);
    drop(prepared);
    assert_eq!(accounts.chat.usage().retained, before.retained);
    drop(child);
    assert_eq!(accounts.chat.usage().retained, 0);
    assert_eq!(accounts.chat.usage().expanded, before.expanded);
    assert_eq!(accounts.chat.usage().spent_fuel, before.spent_fuel);
}

#[test]
fn provisional_reply_work_cannot_spend_the_final_parse_reservation() {
    let accounts = Accounts::isolated(AccountScope::Reply).unwrap();
    let mut reply = ReplyWork::new(&accounts).unwrap();
    assert_eq!(accounts.chat.usage().reserved_fuel, 600_000);
    reply.provisional(1_400_000).unwrap();
    assert_eq!(reply.provisional(1), Err(Refused::UnitFuel));
    let mut finalized = false;
    reply
        .finish(|| {
            finalized = true;
            ((), 2)
        })
        .unwrap();
    assert!(finalized);
    assert_eq!(accounts.chat.usage().spent_fuel, 1_400_004);
    assert_eq!(accounts.chat.usage().reserved_fuel, 0);
}

#[test]
fn prepared_retained_bytes_include_the_ownership_ledger() {
    for (format, bytes) in [
        (crate::data::Format::Json, &br#"{"a":["hello",7]}"#[..]),
        (crate::data::Format::Csv, &b"a,b\nhello,7\nworld,8"[..]),
    ] {
        let accounts = Accounts::isolated(AccountScope::Interface).unwrap();
        let mut prepared = None;
        let measured = allocation_counter::measure(|| {
            prepared = Some(
                crate::data::prepare_accounted(
                    &accounts,
                    UnitKind::DataPreparation,
                    bytes,
                    format,
                    &Default::default(),
                    crate::SourceSpan {
                        line: 1,
                        column: 1,
                        end_line: 1,
                        end_column: 1,
                    },
                )
                .unwrap(),
            );
        });
        assert!(measured.bytes_current > 0);
        assert!(
            measured.bytes_current as u64 <= accounts.chat.usage().retained,
            "retained requested {}, charged {}",
            measured.bytes_current,
            accounts.chat.usage().retained
        );
        drop(prepared);
        assert_eq!(accounts.chat.usage().retained, 0);
    }
}

#[test]
fn tree_retention_counts_physical_containers_and_shared_frames_release_once() {
    use crate::eval::tree::{Tree, TreeNode};
    let accounts = Accounts::isolated(AccountScope::Interface).unwrap();
    let mut frame = None;
    let measured = allocation_counter::measure(|| {
        let tree = Tree {
            blocks: vec![TreeNode::Text(crate::DisplayText::new("hello"))],
        };
        let retention = accounts.retain_tree(&tree).unwrap();
        frame = Some((Arc::new(tree), retention));
    });
    let charged = accounts.chat.usage().retained;
    assert!(measured.bytes_current > 0);
    assert!(measured.bytes_current as u64 <= charged);
    let clone = frame.clone();
    assert!(Arc::ptr_eq(
        &frame.as_ref().unwrap().0,
        &clone.as_ref().unwrap().0
    ));
    drop(frame);
    assert_eq!(accounts.chat.usage().retained, charged);
    drop(clone);
    assert_eq!(accounts.chat.usage().retained, 0);
}

impl super::Accounts {
    // Unit tests can observe the local scope without exposing a production handle.
    pub(crate) fn local_usage_for_tests(&self) -> super::AccountUsage {
        self.local.usage()
    }
}
