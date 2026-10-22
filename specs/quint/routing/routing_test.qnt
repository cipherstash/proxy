// -*- mode: Bluespec; -*-
/**
 * Scenario tests for routing.qnt. Each test drives one concrete execution and
 * checks the outcome, so the witnesses the requirements depend on are reached
 * deterministically rather than by sampling.
 */

/// Scenarios against the behaviour as built (cipherstash/proxy#484).
module routingAsBuiltTest {
  import routing(
    COLS = Set("a", "b"),
    NAMES = Set("s"),
    CONCURRENT_DDL = true,
    PG_DOMAIN_CHECK = false,
    UNCHECKED_WRITES = true,
    BIND_RECHECKS_MAPPING = false,
    UNSAFE_SETTING = true,
  ).* from "./routing"

  pure val aEncrypted = Map("a" -> Encrypted, "b" -> Native)
  pure val allNative = Map("a" -> Native, "b" -> Native)

  pure def stmt(op: Op, col: str, kind: Kind): Stmt = { op: op, col: col, kind: kind }

  run mappedWriteStoresCiphertextTest =
    initWith(aEncrypted, Set(), false)
      .then(simpleQuery([stmt(Write, "a", Checked)]))
      .expect(and {
        outcome == Executed(Set(Mapped)),
        atRest == Set({ payload: Cipher, unsafe: false }),
        safety,
      })

  run nativeWriteForwardedTest =
    initWith(aEncrypted, Set(), false)
      .then(simpleQuery([stmt(Write, "b", Checked)]))
      .expect(and {
        outcome == Executed(Set(Forward(NothingEncrypted))),
        lastSent.forall(w => w.payload == Plain and w.routedView == VNative),
        atRest == Set(),
      })

  run coverageGapWriteRefusedTest =
    // A write never falls back while encrypted columns are configured, even
    // to a native column and with mapping errors off.
    initWith(aEncrypted, Set(), false)
      .then(simpleQuery([stmt(Write, "b", Gap)]))
      .expect(outcome == Refused(Set(Refuse)))

  run coverageGapWriteFallsBackWithoutEncryptionTest =
    initWith(allNative, Set(), false)
      .then(simpleQuery([stmt(Write, "a", Gap)]))
      .expect(outcome == Executed(Set(Forward(Fallback))))

  run coverageGapReadFallsBackTest =
    initWith(aEncrypted, Set(), false)
      .then(simpleQuery([stmt(Read, "a", Gap)]))
      .expect(and { readFellBack, lastSent == Set() })

  run coverageGapReadRefusedWithMappingErrorsTest =
    initWith(aEncrypted, Set(), true)
      .then(simpleQuery([stmt(Read, "a", Gap)]))
      .expect(statementRefused)

  run unresolvableWriteRefusedTest =
    // CIP-5102: the type checker never saw the table's columns.
    initWith(aEncrypted, Set("b"), false)
      .then(simpleQuery([stmt(Write, "b", Checked)]))
      .expect(outcome == Refused(Set(Refuse)))

  run batchRuleRefusesMappedWriteBesideFallbackTest =
    // The fallback would forward the mapped INSERT as written.
    initWith(aEncrypted, Set(), false)
      .then(simpleQuery([stmt(Write, "a", Checked), stmt(Read, "b", Gap)]))
      .expect(and { batchRuleFired, lastSent == Set(), atRest == Set() })

  run preparedWriteMappedAtBindTest =
    initWith(aEncrypted, Set(), false)
      .then(parse("s", stmt(Write, "a", Checked)))
      .then(bindExecute("s"))
      .expect(and { outcome == Executed(Set(Mapped)), atRest == Set({ payload: Cipher, unsafe: false }) })

  run unknownStatementRefusedAtBindTest =
    initWith(aEncrypted, Set(), false)
      .then(bindExecute("s"))
      .expect(unknownBindRefused)

  run closedStatementRefusedAtBindTest =
    initWith(aEncrypted, Set(), false)
      .then(parse("s", stmt(Write, "a", Checked)))
      .then(close("s"))
      .then(bindExecute("s"))
      .expect(unknownBindRefused)

  run refusedParseForgetsTheNameTest =
    initWith(aEncrypted, Set(), false)
      .then(parse("s", stmt(Write, "a", Checked)))
      .then(parse("s", stmt(Write, "a", Gap)))
      .then(bindExecute("s"))
      .expect(unknownBindRefused)

  run unsafeDisableMappingStoresPlaintextTest =
    initWith(aEncrypted, Set(), false)
      .then(setUnsafe(true))
      .then(simpleQuery([stmt(Write, "a", Checked)]))
      .expect(and { unsafePlaintextStored, safety })

  /// Finding F3: a statement prepared while mapping was disabled keeps its
  /// passthrough route after mapping is enabled again.
  run f3UnsafeParseBindsPlaintextTest =
    initWith(aEncrypted, Set(), false)
      .then(setUnsafe(true))
      .then(parse("s", stmt(Write, "a", Checked)))
      .then(setUnsafe(false))
      .then(bindExecute("s"))
      .expect(and {
        outcome == Executed(Set(Forward(MappingDisabled))),
        not(noPlaintextToKnownEncrypted),
        not(noPlaintextAtRest),
      })

  /// Finding F1: a statement prepared against a native column keeps its
  /// route after another connection makes the column encrypted.
  run f1StalePreparedRouteTest =
    initWith(allNative, Set(), false)
      .then(parse("s", stmt(Write, "a", Checked)))
      .then(concurrentDdl("a", Encrypted))
      .then(bindExecute("s"))
      .expect(and {
        noPlaintextToKnownEncrypted,
        not(noPlaintextToCurrentlyEncrypted),
        not(noPlaintextAtRest),
      })

  /// Finding F1 inside a transaction: the pinned snapshot is stale for a
  /// simple Query too.
  run f1PinnedSnapshotTest =
    initWith(allNative, Set(), false)
      .then(begin)
      .then(concurrentDdl("a", Encrypted))
      .then(simpleQuery([stmt(Write, "a", Checked)]))
      .expect(and {
        outcome == Executed(Set(Forward(NothingEncrypted))),
        not(noPlaintextAtRest),
      })

  /// After the transaction, the connection adopts the published snapshot.
  run snapshotAdoptedAfterTransactionTest =
    initWith(allNative, Set(), false)
      .then(begin)
      .then(concurrentDdl("a", Encrypted))
      .then(endTx)
      .then(simpleQuery([stmt(Write, "a", Checked)]))
      .expect(and { outcome == Executed(Set(Mapped)), safety })

  /// Finding F2: a statement kind Proxy never type checks writes as written.
  run f2UncheckedWriteTest =
    initWith(aEncrypted, Set(), false)
      .then(simpleQuery([stmt(Write, "a", Unchecked)]))
      .expect(and {
        outcome == Executed(Set(Forward(NotTypeChecked))),
        not(noPlaintextToKnownEncrypted),
      })
}

/// Scenarios against the candidate fix for D2, with PostgreSQL's domain check.
module routingCandidateTest {
  import routing(
    COLS = Set("a", "b"),
    NAMES = Set("s"),
    CONCURRENT_DDL = true,
    PG_DOMAIN_CHECK = true,
    UNCHECKED_WRITES = false,
    BIND_RECHECKS_MAPPING = true,
    UNSAFE_SETTING = true,
  ).* from "./routing"

  pure val aEncrypted = Map("a" -> Encrypted, "b" -> Native)
  pure val allNative = Map("a" -> Native, "b" -> Native)

  pure def stmt(op: Op, col: str, kind: Kind): Stmt = { op: op, col: col, kind: kind }

  run unsafeParseRefusedAtBindTest =
    initWith(aEncrypted, Set(), false)
      .then(setUnsafe(true))
      .then(parse("s", stmt(Write, "a", Checked)))
      .then(setUnsafe(false))
      .then(bindExecute("s"))
      .expect(and { outcome == Refused(Set(Refuse)), safety })

  run unsafeParsePassesThroughWhileUnsafeTest =
    // Forwarded as written; the domain check then rejects the plaintext.
    initWith(aEncrypted, Set(), false)
      .then(setUnsafe(true))
      .then(parse("s", stmt(Write, "a", Checked)))
      .then(bindExecute("s"))
      .expect(and {
        lastSent.forall(w => w.payload == Plain and w.unsafe),
        dbRejected,
        safety,
      })

  run domainCheckRejectsStaleRouteTest =
    initWith(allNative, Set(), false)
      .then(parse("s", stmt(Write, "a", Checked)))
      .then(concurrentDdl("a", Encrypted))
      .then(bindExecute("s"))
      .expect(and { dbRejected, not(noPlaintextToCurrentlyEncrypted), safety })
}
