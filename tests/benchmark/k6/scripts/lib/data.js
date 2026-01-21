// JSONB payload generators for k6 benchmarks
// Matches integration test fixtures in common.rs

export function randomId() {
  return Math.floor(Math.random() * Number.MAX_SAFE_INTEGER);
}

export function generateStandardJsonb(id) {
  return {
    id: id,
    string: 'hello',
    number: 42,
    nested: {
      number: 1815,
      string: 'world',
    },
    array_string: ['hello', 'world'],
    array_number: [42, 84],
  };
}

export function generateLargeJsonb(id) {
  // Credit report structure: 50 tradelines x 24 months = ~500KB
  const tradelines = [];
  for (let i = 0; i < 50; i++) {
    const history = [];
    for (let m = 0; m < 24; m++) {
      history.push({
        month: m + 1,
        balance: Math.floor(Math.random() * 10000),
        status: 'current',
        payment: Math.floor(Math.random() * 500),
      });
    }
    tradelines.push({
      creditor: `Creditor ${i}`,
      account_number: `ACCT${id}${i}`.padStart(16, '0'),
      account_type: 'revolving',
      opened_date: '2020-01-15',
      credit_limit: 5000 + (i * 100),
      current_balance: Math.floor(Math.random() * 5000),
      payment_history: history,
    });
  }

  return {
    id: id,
    report_id: `RPT-${id}`,
    subject: {
      name: 'Test Subject',
      ssn_last4: '1234',
      dob: '1990-01-01',
    },
    tradelines: tradelines,
    inquiries: [
      { date: '2024-01-01', creditor: 'Bank A' },
      { date: '2024-02-15', creditor: 'Bank B' },
    ],
    public_records: [],
    score: 750,
  };
}
