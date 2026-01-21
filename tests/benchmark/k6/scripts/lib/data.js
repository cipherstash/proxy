// JSONB payload generators for k6 benchmarks
// Matches integration test fixtures in common.rs

// PostgreSQL int4 (serial) max value - prevents "out of range for type integer" errors
const MAX_INT4 = 2147483647;

export function randomId() {
  return Math.floor(Math.random() * MAX_INT4);
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

// Helper: generate random date string within range
function randomDate(startYear, endYear) {
  const year = startYear + Math.floor(Math.random() * (endYear - startYear + 1));
  const month = String(Math.floor(Math.random() * 12) + 1).padStart(2, '0');
  const day = String(Math.floor(Math.random() * 28) + 1).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

// Helper: generate random alphanumeric string
function randomAlphanumeric(length) {
  const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
  let result = '';
  for (let i = 0; i < length; i++) {
    result += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return result;
}

// Account type options for tradelines
const ACCOUNT_TYPES = ['revolving', 'installment', 'mortgage', 'open', 'collection'];
const PAYMENT_STATUSES = ['current', 'late_30', 'late_60', 'late_90', 'late_120', 'charged_off'];
const INDUSTRY_CODES = ['bank', 'credit_union', 'finance_company', 'retail', 'utility', 'medical'];

/**
 * Generate extract payload (~250KB)
 * Structure based on processed credit report data with:
 * - 130 tradelines with 15 payment snapshots each
 * - 45 inquiries with industry data
 * - Credit scores with score reasons
 */
export function generateExtractPayload(id) {
  // Generate 130 tradelines (tuned for ~250KB)
  const tradelines = [];
  for (let i = 0; i < 130; i++) {
    // 15 payment snapshots per tradeline
    const paymentHistory = [];
    for (let m = 0; m < 15; m++) {
      paymentHistory.push({
        period: `2023-${String(m + 1).padStart(2, '0')}`,
        balance: Math.floor(Math.random() * 50000),
        status: PAYMENT_STATUSES[Math.floor(Math.random() * PAYMENT_STATUSES.length)],
        scheduledPayment: Math.floor(Math.random() * 2000),
        actualPayment: Math.floor(Math.random() * 2000),
        pastDueAmount: Math.floor(Math.random() * 1000),
      });
    }

    tradelines.push({
      tradelineId: `TL-${id}-${i}`,
      creditorName: `Creditor ${String.fromCharCode(65 + (i % 26))}${Math.floor(i / 26)}`,
      accountNumber: randomAlphanumeric(16),
      accountType: ACCOUNT_TYPES[i % ACCOUNT_TYPES.length],
      accountStatus: i % 10 === 0 ? 'closed' : 'open',
      openedDate: randomDate(2010, 2022),
      closedDate: i % 10 === 0 ? randomDate(2022, 2024) : null,
      creditLimit: 1000 + (i * 500),
      highestBalance: 500 + Math.floor(Math.random() * 10000),
      currentBalance: Math.floor(Math.random() * 8000),
      monthlyPayment: 50 + Math.floor(Math.random() * 500),
      lastActivityDate: randomDate(2023, 2024),
      paymentHistory: paymentHistory,
      termsMonths: [12, 24, 36, 48, 60][i % 5],
      originalAmount: 1000 + (i * 1000),
      responsibilityCode: ['individual', 'joint', 'authorized'][i % 3],
    });
  }

  // Generate 45 inquiries (tuned for ~250KB)
  const inquiries = [];
  for (let i = 0; i < 45; i++) {
    inquiries.push({
      inquiryId: `INQ-${id}-${i}`,
      inquiryDate: randomDate(2022, 2024),
      creditorName: `Bank ${String.fromCharCode(65 + (i % 26))}`,
      industryCode: INDUSTRY_CODES[i % INDUSTRY_CODES.length],
      inquiryType: i % 3 === 0 ? 'hard' : 'soft',
      purposeCode: ['credit_card', 'auto_loan', 'mortgage', 'personal_loan'][i % 4],
    });
  }

  // Generate score reasons
  const scoreReasons = [
    { code: 'R01', description: 'Length of credit history' },
    { code: 'R02', description: 'Number of accounts with balances' },
    { code: 'R03', description: 'Proportion of balances to credit limits' },
    { code: 'R04', description: 'Recent account activity' },
    { code: 'R05', description: 'Number of recent inquiries' },
  ];

  return {
    reportId: `RPT-${id}`,
    generatedAt: new Date().toISOString(),
    consumer: {
      consumerId: `CON-${id}`,
      firstName: `FirstName${id % 1000}`,
      lastName: `LastName${id % 1000}`,
      dateOfBirth: randomDate(1950, 2000),
      ssnMasked: `XXX-XX-${String(id % 10000).padStart(4, '0')}`,
    },
    scores: [
      {
        scoreType: 'primary',
        scoreValue: 300 + Math.floor(Math.random() * 550),
        scoreDate: randomDate(2024, 2024),
        scoreReasons: scoreReasons.slice(0, 4),
      },
      {
        scoreType: 'industry',
        scoreValue: 300 + Math.floor(Math.random() * 550),
        scoreDate: randomDate(2024, 2024),
        scoreReasons: scoreReasons.slice(1, 5),
      },
    ],
    tradelines: tradelines,
    inquiries: inquiries,
    publicRecords: [],
    collections: [],
    summary: {
      totalAccounts: tradelines.length,
      openAccounts: tradelines.filter(t => t.accountStatus === 'open').length,
      closedAccounts: tradelines.filter(t => t.accountStatus === 'closed').length,
      totalBalance: tradelines.reduce((sum, t) => sum + t.currentBalance, 0),
      totalCreditLimit: tradelines.reduce((sum, t) => sum + t.creditLimit, 0),
      hardInquiries: inquiries.filter(i => i.inquiryType === 'hard').length,
      softInquiries: inquiries.filter(i => i.inquiryType === 'soft').length,
    },
  };
}

/**
 * Generate full payload (~500KB)
 * Structure based on raw credit bureau data with:
 * - 330 consumer records (addresses, employers, phone numbers)
 * - 750 processing records with metadata
 */
export function generateFullPayload(id) {
  // Generate 110 address records (tuned for ~500KB)
  const addresses = [];
  for (let i = 0; i < 110; i++) {
    addresses.push({
      addressId: `ADDR-${id}-${i}`,
      addressLine1: `${100 + i} Street ${String.fromCharCode(65 + (i % 26))}`,
      addressLine2: i % 5 === 0 ? `Apt ${i}` : null,
      city: `City${i % 50}`,
      state: ['CA', 'TX', 'NY', 'FL', 'IL', 'PA', 'OH', 'GA', 'NC', 'MI'][i % 10],
      zipCode: `${10000 + (i * 100)}`,
      addressType: ['current', 'previous', 'mailing'][i % 3],
      reportedDate: randomDate(2015, 2024),
      verifiedDate: i % 2 === 0 ? randomDate(2023, 2024) : null,
      sourceCode: `SRC${i % 10}`,
      residencyMonths: Math.floor(Math.random() * 120),
    });
  }

  // Generate 110 employer records (tuned for ~500KB)
  const employers = [];
  for (let i = 0; i < 110; i++) {
    employers.push({
      employerId: `EMP-${id}-${i}`,
      employerName: `Company ${String.fromCharCode(65 + (i % 26))}${Math.floor(i / 26)} Inc`,
      occupation: ['engineer', 'manager', 'analyst', 'director', 'specialist'][i % 5],
      industry: ['technology', 'finance', 'healthcare', 'retail', 'manufacturing'][i % 5],
      employmentStatus: ['employed', 'self_employed', 'unemployed', 'retired'][i % 4],
      startDate: randomDate(2010, 2022),
      endDate: i % 4 === 2 ? randomDate(2022, 2024) : null,
      income: 30000 + Math.floor(Math.random() * 170000),
      incomeFrequency: ['annual', 'monthly', 'weekly'][i % 3],
      verifiedDate: i % 3 === 0 ? randomDate(2023, 2024) : null,
      sourceCode: `SRC${i % 10}`,
    });
  }

  // Generate 110 phone number records (tuned for ~500KB)
  const phoneNumbers = [];
  for (let i = 0; i < 110; i++) {
    phoneNumbers.push({
      phoneId: `PHN-${id}-${i}`,
      phoneNumber: `${200 + (i % 800)}-${100 + (i % 900)}-${1000 + (i % 9000)}`,
      phoneType: ['mobile', 'home', 'work'][i % 3],
      isPrimary: i === 0,
      reportedDate: randomDate(2018, 2024),
      verifiedDate: i % 2 === 0 ? randomDate(2023, 2024) : null,
      sourceCode: `SRC${i % 10}`,
    });
  }

  // Generate 750 processing records (tuned for ~500KB)
  const processingRecords = [];
  for (let i = 0; i < 750; i++) {
    processingRecords.push({
      recordId: `PROC-${id}-${i}`,
      recordType: ['tradeline', 'inquiry', 'public_record', 'collection', 'consumer_statement'][i % 5],
      sourceId: `SOURCE-${i % 20}`,
      sourceType: ['bureau', 'creditor', 'public', 'consumer'][i % 4],
      receivedAt: randomDate(2020, 2024) + 'T' + String(Math.floor(Math.random() * 24)).padStart(2, '0') + ':' +
                  String(Math.floor(Math.random() * 60)).padStart(2, '0') + ':' +
                  String(Math.floor(Math.random() * 60)).padStart(2, '0') + 'Z',
      processedAt: randomDate(2020, 2024) + 'T' + String(Math.floor(Math.random() * 24)).padStart(2, '0') + ':' +
                   String(Math.floor(Math.random() * 60)).padStart(2, '0') + ':' +
                   String(Math.floor(Math.random() * 60)).padStart(2, '0') + 'Z',
      status: ['processed', 'pending', 'error', 'archived'][i % 4],
      validationScore: Math.floor(Math.random() * 100),
      matchConfidence: Math.random(),
      metadata: {
        version: `v${1 + (i % 5)}.${i % 10}.0`,
        checksum: randomAlphanumeric(32),
        encoding: 'UTF-8',
        compressionType: i % 3 === 0 ? 'gzip' : 'none',
        sizeBytes: 100 + Math.floor(Math.random() * 10000),
        processingTimeMs: Math.floor(Math.random() * 1000),
        retryCount: i % 10 === 0 ? Math.floor(Math.random() * 3) : 0,
        priority: ['low', 'medium', 'high', 'critical'][i % 4],
        tags: [`tag${i % 10}`, `category${i % 5}`],
      },
      rawData: {
        originalFormat: ['xml', 'json', 'csv', 'fixed_width'][i % 4],
        fieldCount: 10 + (i % 50),
        nullFields: i % 10,
        warningCount: i % 5,
        transformations: [`transform_${i % 8}`],
      },
    });
  }

  // Generate dispute records
  const disputes = [];
  for (let i = 0; i < 20; i++) {
    disputes.push({
      disputeId: `DISP-${id}-${i}`,
      disputeType: ['accuracy', 'identity', 'fraud', 'duplicate'][i % 4],
      disputeStatus: ['open', 'investigating', 'resolved', 'rejected'][i % 4],
      filedDate: randomDate(2022, 2024),
      resolvedDate: i % 4 === 2 || i % 4 === 3 ? randomDate(2023, 2024) : null,
      relatedRecordId: `PROC-${id}-${i * 10}`,
      description: `Dispute regarding record ${i}`,
      resolution: i % 4 === 2 ? 'corrected' : i % 4 === 3 ? 'verified_accurate' : null,
    });
  }

  return {
    rawReportId: `RAW-${id}`,
    bureauCode: ['experian', 'equifax', 'transunion'][id % 3],
    pullDate: new Date().toISOString(),
    consumer: {
      consumerId: `CON-${id}`,
      firstName: `FirstName${id % 1000}`,
      middleName: id % 3 === 0 ? `MiddleName${id % 100}` : null,
      lastName: `LastName${id % 1000}`,
      suffix: id % 20 === 0 ? ['Jr', 'Sr', 'III'][id % 3] : null,
      dateOfBirth: randomDate(1950, 2000),
      ssnFull: `${String(100 + (id % 900)).padStart(3, '0')}-${String(10 + (id % 90)).padStart(2, '0')}-${String(id % 10000).padStart(4, '0')}`,
      addresses: addresses,
      employers: employers,
      phoneNumbers: phoneNumbers,
    },
    processingRecords: processingRecords,
    disputes: disputes,
    metadata: {
      version: '2.0.0',
      schemaVersion: '1.5.0',
      generatedBy: 'benchmark-generator',
      processingNode: `node-${id % 10}`,
      totalRecords: processingRecords.length,
      totalConsumerRecords: addresses.length + employers.length + phoneNumbers.length,
      checksums: {
        consumer: randomAlphanumeric(64),
        processing: randomAlphanumeric(64),
        disputes: randomAlphanumeric(64),
      },
    },
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
