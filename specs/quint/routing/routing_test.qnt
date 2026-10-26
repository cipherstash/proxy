// -*- mode: Bluespec; -*-
/**
 * Scenario tests for routing.qnt. Each test drives one concrete execution and
 * checks the outcome, so the witnesses the requirements depend on are reached
 * deterministically rather than by sampling.
 */

/// Scenarios against the behaviour as built (cipherstash/proxy#484). The
/// configuration enables concurrent DDL and `Opaque` writes, unlike the
/// `asBuilt` instance, so that the F1 and F2 scenarios can run.
module routingScenarioTest {
  import routing(
    COLS = Set("a", "b"),
    NAMES = Set("s"),
    CONCURRENT_DDL = true,
    PG_DOMAIN_CHECK = false,
    UNCHECKED_WRITES = true,
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
      .expect(and { fellBack, lastSent == Set() })

  run coverageGapReadRefusedWithMappingErrorsTest =
    initWith(aEncrypted, Set(), true)
      .then(simpleQuery([stmt(Read, "a", Gap)]))
      .expect(statementRefused)

  run unresolvableWriteRefusedTest =
    // #486: the type checker never saw the table's columns.
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
      .expect(and { outcome == BoundExecuted(Mapped), atRest == Set({ payload: Cipher, unsafe: false }) })

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

  run refusedParseLeavesTheNameUnusedTest =
    initWith(aEncrypted, Set(), false)
      .then(parse("s", stmt(Write, "a", Gap)))
      .expect(statementRefused)
      .then(bindExecute("s"))
      .expect(unknownBindRefused)

  run parseOfANameInUseIsLeftOutTest =
    // PostgreSQL rejects it and Proxy restores the statement it held.
    initWith(aEncrypted, Set(), false)
      .then(parse("s", stmt(Write, "a", Checked)))
      .then(parse("s", stmt(Write, "b", Checked)))
      .fail()

  run unknownStatementForwardedWhileUnsafeTest =
    // `bind_handler` passes the Bind through before `is_prepared`.
    initWith(aEncrypted, Set(), false)
      .then(setUnsafe(true))
      .then(bindExecute("s"))
      .expect(and { dbRejected, lastSent == Set() })

  run copyFromIntoEncryptedRefusedTest =
    initWith(aEncrypted, Set(), false)
      .then(simpleQuery([stmt(Write, "a", CopyFrom)]))
      .expect(and { outcome == Refused(Set(Refuse)), lastSent == Set() })

  run copyFromIntoUnresolvableRefusedTest =
    initWith(aEncrypted, Set("b"), false)
      .then(simpleQuery([stmt(Write, "b", CopyFrom)]))
      .expect(outcome == Refused(Set(Refuse)))

  run copyFromIntoNativeForwardedTest =
    initWith(aEncrypted, Set(), false)
      .then(simpleQuery([stmt(Write, "b", CopyFrom)]))
      .expect(and { outcome == Executed(Set(Forward(NothingEncrypted))), safety })

  run copyFromWithoutEncryptionForwardedTest =
    // `check_copy_into` runs only while encrypted columns are configured.
    initWith(allNative, Set("a"), false)
      .then(simpleQuery([stmt(Write, "a", CopyFrom)]))
      .expect(outcome == Executed(Set(Forward(NothingEncrypted))))

  run preparedCopyFromSettingsChangedRefusedTest =
    // `requires_schema` covers `COPY ... FROM`, so the Bind check does too.
    initWith(aEncrypted, Set(), false)
      .then(setUnsafe(true))
      .then(parse("s", stmt(Write, "a", CopyFrom)))
      .then(setUnsafe(false))
      .then(bindExecute("s"))
      .expect(and { settingsChangedRefused, safety })

  run nestedWriteRefusedTest =
    initWith(aEncrypted, Set(), false)
      .then(simpleQuery([stmt(Write, "b", NestedWrite)]))
      .expect(outcome == Refused(Set(Refuse)))

  run preparedNestedWriteKeepsItsParseSettingTest =
    // `requires_schema` leaves nested writes out of the Bind check, so the
    // Bind is forwarded; its values were sent with mapping disabled (A6).
    initWith(aEncrypted, Set(), false)
      .then(setUnsafe(true))
      .then(parse("s", stmt(Write, "a", NestedWrite)))
      .then(setUnsafe(false))
      .then(bindExecute("s"))
      .expect(and {
        outcome == BoundExecuted(Forward(MappingDisabled)),
        lastSent.forall(w => w.unsafe),
        safety,
      })

  run nestedWriteWithoutEncryptionForwardedTest =
    initWith(allNative, Set(), false)
      .then(simpleQuery([stmt(Write, "a", NestedWrite)]))
      .expect(outcome == Executed(Set(Forward(NotTypeChecked))))

  run closeTransactionAndSettingWitnessesTest =
    initWith(aEncrypted, Set(), false)
      .then(parse("s", stmt(Read, "a", Checked)))
      .then(close("s"))
      .expect(statementClosed)
      .then(begin)
      .then(endTx)
      .expect(transactionEnded)
      .then(setUnsafe(true))
      .expect(mappingDisabled)

  run unsafeDisableMappingStoresPlaintextTest =
    initWith(aEncrypted, Set(), false)
      .then(setUnsafe(true))
      .then(simpleQuery([stmt(Write, "a", Checked)]))
      .expect(and { unsafePlaintextStored, safety })

  /// A statement prepared while mapping was disabled is refused at Bind once
  /// mapping is enabled again (`MappingError::PreparedSettingsChanged`).
  run settingsChangedRefusedAfterUnsafeParseTest =
    initWith(aEncrypted, Set(), false)
      .then(setUnsafe(true))
      .then(parse("s", stmt(Write, "a", Checked)))
      .then(setUnsafe(false))
      .then(bindExecute("s"))
      .expect(and { settingsChangedRefused, lastSent == Set(), safety })

  /// The other direction: prepared with mapping on, bound with it off.
  run settingsChangedRefusedAfterMappedParseTest =
    initWith(aEncrypted, Set(), false)
      .then(parse("s", stmt(Write, "a", Checked)))
      .then(setUnsafe(true))
      .then(bindExecute("s"))
      .expect(and { settingsChangedRefused, lastSent == Set(), atRest == Set() })

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

  /// Finding F2: a statement Proxy cannot inspect writes as written.
  run f2OpaqueWriteTest =
    initWith(aEncrypted, Set(), false)
      .then(simpleQuery([stmt(Write, "a", Opaque)]))
      .expect(and {
        outcome == Executed(Set(Forward(NotTypeChecked))),
        not(noPlaintextToKnownEncrypted),
      })
}

/// Scenarios with PostgreSQL's domain check, as in production.
module routingDomainCheckTest {
  import routing(
    COLS = Set("a", "b"),
    NAMES = Set("s"),
    CONCURRENT_DDL = true,
    PG_DOMAIN_CHECK = true,
    UNCHECKED_WRITES = false,
  ).* from "./routing"

  pure val aEncrypted = Map("a" -> Encrypted, "b" -> Native)
  pure val allNative = Map("a" -> Native, "b" -> Native)

  pure def stmt(op: Op, col: str, kind: Kind): Stmt = { op: op, col: col, kind: kind }

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
