// -*- mode: Bluespec; -*-
/**
 * Statement routing: may a client's plaintext reach an encrypted column?
 *
 * A design-level model of how Proxy decides, for each statement, whether to
 * map it (encrypt and rewrite), forward it as written, or refuse it, and of
 * what the database ends up storing as a result. See requirements.md for the
 * requirements, sources and assumptions behind every definition here.
 *
 * The routing policy is the refuse-by-default `Frontend::route` from
 * cipherstash/proxy#484 (packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs).
 *
 * Scope: one client connection, a database with a few columns, an optional
 * concurrent DDL actor (another connection whose DDL commits and publishes),
 * simple Query batches, Parse and Bind/Execute, Close, transactions,
 * `COPY ... FROM`, and `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING`.
 */
module routing {
  // ---------------------------------------------------------------------
  // Parameters
  // ---------------------------------------------------------------------

  /// Column names, each standing for one `table.column`.
  const COLS: Set[str]
  /// Prepared statement names the client may use.
  const NAMES: Set[str]
  /// Another connection may commit DDL that changes a column's type (and so
  /// whether it is encrypted) at any point.
  const CONCURRENT_DDL: bool
  /// PostgreSQL rejects a plaintext value written to an EQL domain-typed
  /// column (type mismatch or the domain's CHECK constraint). Assumption A4.
  const PG_DOMAIN_CHECK: bool
  /// Statements Proxy cannot inspect (`DO`, `CALL`, SQL `EXECUTE`) may carry
  /// values for a column. Assumption A3.
  const UNCHECKED_WRITES: bool

  // ---------------------------------------------------------------------
  // Types
  // ---------------------------------------------------------------------

  /// A column's type in the database. Under EQL v3 an encrypted column is a
  /// column whose type is an EQL domain.
  type ColType = Native | Encrypted

  /// How Proxy's loaded schema sees one column.
  type View = VNative | VEncrypted | VUnknown

  type Op = Read | Write

  /// What happens when the statement meets EQL Mapper.
  /// `Checked`: Proxy type checks it and the check succeeds whenever the
  /// column is resolvable. `Gap`: a coverage gap, the type check fails on
  /// valid SQL. `CopyFrom`: a `COPY ... FROM` (`copies_into_table`), checked
  /// against the schema but never type checked. `NestedWrite`: a write
  /// nested in a statement `requires_type_check` excludes, which
  /// `writes_data` finds, such as `COPY (INSERT ...) TO`. `Opaque`: a
  /// statement whose values Proxy cannot see, such as `DO` or `CALL`.
  type Kind = Checked | Gap | CopyFrom | NestedWrite | Opaque

  type Stmt = { op: Op, col: str, kind: Kind }

  /// Why a statement is forwarded as written (`Forward` in frontend.rs).
  type Reason = MappingDisabled | NotTypeChecked | NothingEncrypted | Fallback

  /// The routing decision (`Route` in frontend.rs, whose `Map` is `Mapped` here because
  /// `Map` is a Quint built-in; refusal is a value rather than an `Err`).
  type Route = Mapped | Forward(Reason) | Refuse

  /// One immutable committed schema snapshot as a connection sees it.
  type Snapshot = { view: str -> View, encConfigured: bool }

  /// What Proxy keeps for a prepared statement: the route its Parse took, the
  /// snapshot it was routed against, and whether mapping was disabled then.
  type Prepared = { stmt: Stmt, route: Route, routedAt: Snapshot, unsafeAtParse: bool }

  type Slot = Empty | Held(Prepared)

  type Payload = Plain | Cipher

  /// A write that reached the database, with what Proxy knew when routing it
  /// and what its current schema says at the moment it is sent.
  type Sent = {
    col: str,
    payload: Payload,
    unsafe: bool,
    routedView: View,
    routedEncConfigured: bool,
    currentView: View,
  }

  /// A value stored in a column that was encrypted when it was stored, and
  /// whether mapping was disabled on purpose when it was written.
  type AtRest = { payload: Payload, unsafe: bool }

  /// The observable result of the last step, for witnesses and tests.
  type Outcome =
    | Idle
    | Executed(Set[Route])
    | BoundExecuted(Route)
    | Closed
    | TxEnded
    | Refused(Set[Route])
    | BatchRefused
    | UnknownStatementRefused
    | SettingsChangedRefused
    | DbRejected

  type Conn = {
    inTx: bool,
    snap: Snapshot,
    unsafe: bool,
    prepared: str -> Slot,
  }

  // ---------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------

  /// Authoritative column types in PostgreSQL.
  var dbType: str -> ColType
  /// Columns whose table Proxy cannot resolve (e.g. a name in two schemas).
  /// Fixed for a run.
  var unresolvable: Set[str]
  /// `mapping_errors_enabled` (production default: false). Fixed for a run.
  var mappingErrors: bool
  /// The latest committed schema snapshot Proxy has published.
  var published: Snapshot
  /// The one client connection.
  var conn: Conn
  /// Writes Proxy forwarded to the database in the last step.
  var lastSent: Set[Sent]
  /// What the encrypted columns hold. Values in native columns are not
  /// tracked, and neither is which encrypted column holds which value.
  var atRest: Set[AtRest]
  var outcome: Outcome

  // ---------------------------------------------------------------------
  // Pure logic
  // ---------------------------------------------------------------------

  /// `COPY ... FROM` and a nested write always write. A statement that is
  /// not type checked and writes nothing is an `Opaque` read.
  pure val ALL_STMTS: Set[Stmt] =
    tuples(Set(Read, Write), COLS, Set(Checked, Gap, CopyFrom, NestedWrite, Opaque))
      .map(((op, col, kind)) => { op: op, col: col, kind: kind })
      .filter(s => s.op == Write or not(s.kind == CopyFrom or s.kind == NestedWrite))

  /// Statements a client may send. Under assumption A3 an `Opaque` statement
  /// carries no values for a column.
  pure val STMTS: Set[Stmt] =
    if (UNCHECKED_WRITES) ALL_STMTS
    else ALL_STMTS.filter(s => not(s.kind == Opaque and s.op == Write))

  /// Two-statement simple Query batches: two different statements that are
  /// type checked. The batch rule concerns only those, and sending the other
  /// kinds alone keeps the state space small enough for TLC.
  pure val BATCHES: Set[List[Stmt]] =
    STMTS.filter(s => s.kind == Checked or s.kind == Gap)
      .powerset()
      .filter(b => b.size() == 2)
      .map(b => b.fold(List(), (acc, s) => acc.append(s)))

  /// `eql_mapper::requires_schema`: the statements whose Bind the settings
  /// check covers.
  pure def requiresSchema(k: Kind): bool =
    k == Checked or k == Gap or k == CopyFrom

  /// Load a committed schema snapshot from the catalog. The encrypt config is
  /// derived from the catalog's domain types, so it covers encrypted columns
  /// whose table name Proxy cannot resolve (assumption A2).
  pure def load(types: str -> ColType, unres: Set[str]): Snapshot = {
    view: COLS.mapBy(c =>
      if (unres.contains(c)) VUnknown
      else if (types.get(c) == Encrypted) VEncrypted
      else VNative),
    encConfigured: COLS.exists(c => types.get(c) == Encrypted),
  }

  /// `Frontend::may_fall_back`: only while mapping errors are off, and never
  /// for a write while encrypted columns are configured.
  pure def mayFallBack(mapErrs: bool, writesEncrypted: bool): bool =
    not(mapErrs) and not(writesEncrypted)

  /// `check_copy_into`: while encrypted columns are configured, a
  /// `COPY ... FROM` into an encrypted column, or into a table Proxy cannot
  /// resolve, is refused.
  pure def copyRefused(v: View, encConfigured: bool): bool =
    encConfigured and v != VNative

  /// `Frontend::route`.
  pure def route(s: Stmt, snap: Snapshot, unsafe: bool, mapErrs: bool): Route =
    val v = snap.view.get(s.col)
    if (unsafe) Forward(MappingDisabled)
    else if (s.kind == CopyFrom)
      if (copyRefused(v, snap.encConfigured)) Refuse else Forward(NothingEncrypted)
    else if (s.kind == NestedWrite)
      if (snap.encConfigured) Refuse else Forward(NotTypeChecked)
    else if (s.kind == Opaque) Forward(NotTypeChecked)
    else
      // An unresolvable table fails the type check like a coverage gap does.
      val typeCheckFails = s.kind == Gap or v == VUnknown
      if (not(typeCheckFails))
        if (v == VEncrypted) Mapped else Forward(NothingEncrypted)
      else
        val writesEncrypted = snap.encConfigured and s.op == Write
        if (mayFallBack(mapErrs, writesEncrypted)) Forward(Fallback) else Refuse

  pure def isFallback(r: Route): bool = r == Forward(Fallback)

  /// The simple Query batch rule: a fallback forwards the whole Query as
  /// written, so it must not carry a statement that needs mapping.
  pure def batchRefused(routes: List[Route]): bool =
    routes.foldl(false, (acc, r) => acc or r == Mapped)
      and routes.foldl(false, (acc, r) => acc or isFallback(r))

  /// The payload a routed write puts on the wire. `asWritten` is true when a
  /// fallback forwards the whole batch as the client wrote it.
  pure def payloadOf(r: Route, asWritten: bool): Payload =
    if (r == Mapped and not(asWritten)) Cipher else Plain

  pure def accepts(types: str -> ColType, w: Sent): bool =
    not(PG_DOMAIN_CHECK and w.payload == Plain and types.get(w.col) == Encrypted)

  /// What one forwarded write records. `routedAt` is the snapshot the route
  /// was decided against, `current` the connection's schema when it is sent.
  pure def sent(s: Stmt, r: Route, asWritten: bool, unsafe: bool,
                routedAt: Snapshot, current: Snapshot): Sent = {
    col: s.col,
    payload: payloadOf(r, asWritten),
    unsafe: unsafe,
    routedView: routedAt.view.get(s.col),
    routedEncConfigured: routedAt.encConfigured,
    currentView: current.view.get(s.col),
  }

  /// The connection's effective schema: pinned for a transaction, otherwise
  /// the latest published snapshot.
  def effective: Snapshot = if (conn.inTx) conn.snap else published

  // ---------------------------------------------------------------------
  // Actions
  // ---------------------------------------------------------------------

  /// Start from given column types, unresolvable columns and
  /// `mapping_errors_enabled`.
  action initWith(types: str -> ColType, unres: Set[str], mapErrs: bool): bool = all {
    dbType' = types,
    unresolvable' = unres,
    mappingErrors' = mapErrs,
    published' = load(types, unres),
    conn' = {
      inTx: false,
      snap: load(types, unres),
      unsafe: false,
      prepared: NAMES.mapBy(_ => Empty),
    },
    lastSent' = Set(),
    atRest' = Set(),
    outcome' = Idle,
  }

  action init = {
    nondet types = COLS.setOfMaps(Set(Native, Encrypted)).oneOf()
    nondet unres = COLS.powerset().oneOf()
    nondet mapErrs = Set(false, true).oneOf()
    initWith(types, unres, mapErrs)
  }

  /// The database applies forwarded writes atomically: a rejected value
  /// rolls the whole message back. `done` is the outcome when it accepts.
  action deliver(writes: Set[Sent], done: Outcome): bool =
    val allAccepted = writes.forall(w => accepts(dbType, w))
    all {
      lastSent' = writes,
      atRest' =
        if (allAccepted)
          atRest.union(writes
            .filter(w => dbType.get(w.col) == Encrypted)
            .map(w => { payload: w.payload, unsafe: w.unsafe }))
        else atRest,
      outcome' = if (allAccepted) done else DbRejected,
    }

  /// A simple Query carrying one or two statements.
  action simpleQuery(stmts: List[Stmt]): bool =
    val snap = effective
    val routes = stmts.foldl(List(), (acc, s) => acc.append(route(s, snap, conn.unsafe, mappingErrors)))
    val routeSet = routes.foldl(Set(), (acc, r) => acc.union(Set(r)))
    val asWritten = routeSet.exists(r => isFallback(r))
    val writes = stmts.indices()
      .filter(i => stmts[i].op == Write)
      .map(i => sent(stmts[i], routes[i], asWritten, conn.unsafe, snap, snap))
    all {
      stmts.length() > 0,
      stmts.foldl(true, (acc, s) => acc and STMTS.contains(s)),
      conn' = conn,
      dbType' = dbType,
      unresolvable' = unresolvable,
      mappingErrors' = mappingErrors,
      published' = published,
      if (routeSet.contains(Refuse))
        all { lastSent' = Set(), atRest' = atRest, outcome' = Refused(routeSet) }
      else if (batchRefused(routes))
        all { lastSent' = Set(), atRest' = atRest, outcome' = BatchRefused }
      else
        deliver(writes, Executed(routeSet)),
    }

  /// Parse of a name not in use: route against the effective schema and
  /// remember the decision. PostgreSQL rejects a Parse of a name in use, and
  /// Proxy then restores the statement it held (`SettingsProtocol::failed`),
  /// so that Parse changes nothing and is left out. A refused Parse leaves
  /// the name unused (`Context::refuse_parse`).
  action parse(name: str, s: Stmt): bool =
    val snap = effective
    val r = route(s, snap, conn.unsafe, mappingErrors)
    all {
      STMTS.contains(s),
      conn.prepared.get(name) == Empty,
      dbType' = dbType,
      unresolvable' = unresolvable,
      mappingErrors' = mappingErrors,
      published' = published,
      lastSent' = Set(),
      atRest' = atRest,
      if (r == Refuse)
        all {
          conn' = conn,
          outcome' = Refused(Set(r)),
        }
      else
        all {
          conn' = { ...conn, prepared: conn.prepared.set(name, Held({ stmt: s, route: r, routedAt: snap, unsafeAtParse: conn.unsafe })) },
          outcome' = Idle,
        },
    }

  /// Bind and Execute a prepared statement, as one step. Bind reuses the
  /// route its Parse took; it does not route again. It refuses a statement
  /// `requires_schema` accepts when the mapping setting differs from the one
  /// at Parse (`SettingsProtocol::bind`, `MappingError::PreparedSettingsChanged`).
  /// `SettingsProtocol::execute` makes the same check between Bind and
  /// Execute, so a `SET` between them, which this step leaves out, is refused
  /// the same way.
  action bindExecute(name: str): bool = all {
    conn' = conn,
    dbType' = dbType,
    unresolvable' = unresolvable,
    mappingErrors' = mappingErrors,
    published' = published,
    match conn.prepared.get(name) {
      | Empty =>
        // With mapping disabled the Bind is forwarded, and PostgreSQL reports
        // the unknown name. Otherwise it is refused, since its values might
        // need encrypting (`Context::is_prepared`).
        if (conn.unsafe)
          all { lastSent' = Set(), atRest' = atRest, outcome' = DbRejected }
        else
          all { lastSent' = Set(), atRest' = atRest, outcome' = UnknownStatementRefused }
      | Held(p) =>
        // `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING` passes the Bind through.
        val r = if (conn.unsafe) Forward(MappingDisabled) else p.route
        // A nested write takes no parameters, so its values crossed Proxy in
        // the Parse's SQL text, under the setting at Parse (assumption A6).
        val valuesUnsafe = if (p.stmt.kind == NestedWrite) p.unsafeAtParse else conn.unsafe
        val writes =
          if (p.stmt.op == Write) Set(sent(p.stmt, r, false, valuesUnsafe, p.routedAt, effective))
          else Set()
        val settingsChanged = requiresSchema(p.stmt.kind) and conn.unsafe != p.unsafeAtParse
        if (settingsChanged)
          all { lastSent' = Set(), atRest' = atRest, outcome' = SettingsChangedRefused }
        else
          deliver(writes, BoundExecuted(r))
    },
  }

  action close(name: str): bool = all {
    conn' = { ...conn, prepared: conn.prepared.set(name, Empty) },
    dbType' = dbType,
    unresolvable' = unresolvable,
    mappingErrors' = mappingErrors,
    published' = published,
    lastSent' = Set(),
    atRest' = atRest,
    outcome' = Closed,
  }

  /// BEGIN pins the latest published snapshot for the transaction.
  action begin: bool = all {
    not(conn.inTx),
    conn' = { ...conn, inTx: true, snap: published },
    dbType' = dbType,
    unresolvable' = unresolvable,
    mappingErrors' = mappingErrors,
    published' = published,
    lastSent' = Set(),
    atRest' = atRest,
    outcome' = Idle,
  }

  /// COMMIT or ROLLBACK. Values already written are kept either way: the
  /// properties are about what reached the database.
  action endTx: bool = all {
    conn.inTx,
    // Outside a transaction the snapshot is unused; reset it to keep the
    // state space small.
    conn' = { ...conn, inTx: false, snap: published },
    dbType' = dbType,
    unresolvable' = unresolvable,
    mappingErrors' = mappingErrors,
    published' = published,
    lastSent' = Set(),
    atRest' = atRest,
    outcome' = TxEnded,
  }

  /// A successfully executed `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING`.
  action setUnsafe(on: bool): bool = all {
    conn' = { ...conn, unsafe: on },
    dbType' = dbType,
    unresolvable' = unresolvable,
    mappingErrors' = mappingErrors,
    published' = published,
    lastSent' = Set(),
    atRest' = atRest,
    outcome' = Idle,
  }

  /// Another connection commits DDL that changes a column's type, and Proxy
  /// publishes the reloaded snapshot. PostgreSQL applies the new type to
  /// this connection's next statement at once; Proxy's effective schema for
  /// this connection catches up only outside a transaction.
  action concurrentDdl(c: str, t: ColType): bool =
    val types = dbType.set(c, t)
    all {
      CONCURRENT_DDL,
      dbType' = types,
      unresolvable' = unresolvable,
      mappingErrors' = mappingErrors,
      published' = load(types, unresolvable),
      conn' = conn,
      lastSent' = Set(),
      atRest' = atRest,
      outcome' = Idle,
    }

  action step = any {
    nondet s1 = STMTS.oneOf()
    simpleQuery([s1]),
    nondet batch = BATCHES.oneOf()
    simpleQuery(batch),
    nondet name = NAMES.oneOf()
    nondet s = STMTS.oneOf()
    parse(name, s),
    nondet name = NAMES.oneOf()
    bindExecute(name),
    nondet name = NAMES.oneOf()
    close(name),
    begin,
    endTx,
    nondet on = Set(false, true).oneOf()
    setUnsafe(on),
    nondet c = COLS.oneOf()
    nondet t = Set(Native, Encrypted).oneOf()
    concurrentDdl(c, t),
  }

  // ---------------------------------------------------------------------
  // Properties (see requirements.md)
  // ---------------------------------------------------------------------

  pure def plainSafe(w: Sent): bool = w.payload == Cipher or w.unsafe

  /// S1: Proxy never forwards plaintext for a column its schema said was
  /// encrypted when it routed the statement.
  val noPlaintextToKnownEncrypted: bool =
    lastSent.forall(w => plainSafe(w) or w.routedView != VEncrypted)

  /// S2: while encrypted columns are configured, Proxy never forwards
  /// plaintext for a column whose table it cannot resolve.
  val noPlaintextToUnresolvable: bool =
    lastSent.forall(w => plainSafe(w) or not(w.routedView == VUnknown and w.routedEncConfigured))

  /// S3: no write stores plaintext in a column that is encrypted when the
  /// write happens, except one made with mapping disabled on purpose. Values
  /// already in a column when DDL later makes it encrypted are not tracked.
  val noPlaintextAtRest: bool =
    atRest.forall(r => r.payload == Cipher or r.unsafe)

  /// S4 (not required; D1 decided): Proxy never forwards plaintext for a column its
  /// schema says is encrypted at the moment it sends the write.
  val noPlaintextToCurrentlyEncrypted: bool =
    lastSent.forall(w => plainSafe(w) or w.currentView != VEncrypted)

  val safety: bool = and {
    noPlaintextToKnownEncrypted,
    noPlaintextToUnresolvable,
    noPlaintextAtRest,
  }

  // ---------------------------------------------------------------------
  // Witnesses: positive predicates for `quint run --witnesses`, which counts
  // the traces that reach each state.
  // ---------------------------------------------------------------------

  /// A mapped write stored ciphertext in an encrypted column.
  val ciphertextStored: bool =
    atRest.exists(r => r.payload == Cipher)

  /// A statement fell back to forwarding as written.
  val fellBack: bool = match outcome {
    | Executed(rs) => rs.contains(Forward(Fallback))
    | _ => false
  }

  /// A statement was refused.
  val statementRefused: bool = match outcome {
    | Refused(_) => true
    | _ => false
  }

  /// The batch rule refused a Query mixing a mapped statement and a fallback.
  val batchRuleFired: bool = outcome == BatchRefused

  /// A Bind was refused because the mapping setting changed since its Parse.
  val settingsChangedRefused: bool = outcome == SettingsChangedRefused

  /// An unknown prepared statement name was refused at Bind.
  val unknownBindRefused: bool = outcome == UnknownStatementRefused

  /// Plaintext was stored on purpose with mapping disabled.
  val unsafePlaintextStored: bool =
    atRest.exists(r => r.payload == Plain and r.unsafe)

  /// PostgreSQL rejected a forwarded write.
  val dbRejected: bool = outcome == DbRejected

  /// A Parse stored a route under a statement name.
  val statementPrepared: bool =
    NAMES.exists(n => conn.prepared.get(n) != Empty)

  /// A transaction is open, so Proxy routes against its pinned snapshot.
  val inTransaction: bool = conn.inTx

  /// Concurrent DDL was published while a transaction kept the old snapshot.
  val txSnapshotStale: bool = conn.inTx and conn.snap != published

  /// A prepared route was decided against a snapshot Proxy no longer uses.
  val preparedRouteStale: bool =
    NAMES.exists(n => match conn.prepared.get(n) {
      | Held(p) => p.routedAt != effective
      | Empty => false
    })

  /// A Bind executed a prepared statement.
  val boundExecuted: bool = match outcome {
    | BoundExecuted(_) => true
    | _ => false
  }

  /// A Close forgot a statement name.
  val statementClosed: bool = outcome == Closed

  /// A transaction ended.
  val transactionEnded: bool = outcome == TxEnded

  /// Mapping is disabled for the connection.
  val mappingDisabled: bool = conn.unsafe
}

/// As built, with a stable schema and PostgreSQL's domain check switched
/// off, so that S3 rests on Proxy alone. S1, S2 and S3 hold.
module asBuilt {
  import routing(
    COLS = Set("a", "b"),
    NAMES = Set("s"),
    CONCURRENT_DDL = false,
    PG_DOMAIN_CHECK = false,
    UNCHECKED_WRITES = false,
  ).*
}

/// Concurrent DDL with PostgreSQL's domain check, as in production.
/// S1, S2 and S3 hold; S4 fails (F1).
module concurrentDdl {
  import routing(
    COLS = Set("a", "b"),
    NAMES = Set("s"),
    CONCURRENT_DDL = true,
    PG_DOMAIN_CHECK = true,
    UNCHECKED_WRITES = false,
  ).*
}

/// Concurrent DDL without PostgreSQL's domain check: S3 fails (F1).
module concurrentDdlNoDomainCheck {
  import routing(
    COLS = Set("a", "b"),
    NAMES = Set("s"),
    CONCURRENT_DDL = true,
    PG_DOMAIN_CHECK = false,
    UNCHECKED_WRITES = false,
  ).*
}

/// Statements Proxy cannot inspect (`DO`, `CALL`) carry values: S1 fails (F2).
module uncheckedWrites {
  import routing(
    COLS = Set("a", "b"),
    NAMES = Set("s"),
    CONCURRENT_DDL = false,
    PG_DOMAIN_CHECK = false,
    UNCHECKED_WRITES = true,
  ).*
}
