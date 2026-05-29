package main

import (
	"context"
	"flag"
	"fmt"
	"log"
	"sync"
	"sync/atomic"
	"time"

	"github.com/google/uuid"
	"github.com/jackc/pgx/v5/pgxpool"
)

func main() {
	var (
		dsn     = flag.String("dsn", "postgres://bloomuser:password@127.0.0.1:6432/pdts_db?sslmode=disable", "Database connection string (DSN)")
		count   = flag.Int("count", 10000, "Number of inserts to perform")
		workers = flag.Int("workers", 10, "Number of concurrent workers")
	)
	flag.Parse()

	if *dsn == "" {
		log.Fatal("DSN is required. Use -dsn flag to provide database connection string")
	}

	ctx := context.Background()

	pool, err := pgxpool.New(ctx, *dsn)
	if err != nil {
		log.Fatal("Failed to create pgx pool:", err)
	}
	defer pool.Close()

	if err := pool.Ping(ctx); err != nil {
		log.Fatal("Failed to ping database:", err)
	}

	log.Printf("Connected to database, inserting %d records with %d workers...", *count, *workers)

	orgID := "539008ae-e1ff-42ed-8a58-e3588befea9d"
	testJSON := []byte(`{
		"reportId": "RPT-2024-001234567890",
		"generatedAt": "2024-01-15T14:30:00Z",
		"consumer": {
			"firstName": "John",
			"lastName": "Doe",
			"dateOfBirth": "1985-06-15",
			"ssn": "XXX-XX-1234",
			"addresses": [
				{
					"type": "current",
					"street": "123 Main Street",
					"city": "Springfield",
					"state": "IL",
					"zipCode": "62701",
					"since": "2020-03-01"
				},
				{
					"type": "previous",
					"street": "456 Oak Avenue",
					"city": "Chicago",
					"state": "IL",
					"zipCode": "60601",
					"since": "2015-08-15"
				}
			],
			"employment": {
				"employer": "Acme Corporation",
				"position": "Software Engineer",
				"income": 95000,
				"since": "2019-01-15"
			}
		},
		"creditScore": {
			"value": 742,
			"model": "FICO8",
			"range": {"min": 300, "max": 850},
			"factors": [
				{"code": "01", "description": "Length of time accounts have been established"},
				{"code": "14", "description": "Number of accounts with delinquency"},
				{"code": "07", "description": "Too many inquiries last 12 months"}
			]
		},
		"accounts": [
			{
				"accountNumber": "XXXX-XXXX-XXXX-4567",
				"creditor": "First National Bank",
				"type": "creditCard",
				"status": "open",
				"openDate": "2018-05-20",
				"creditLimit": 15000,
				"balance": 3250,
				"monthlyPayment": 150,
				"paymentHistory": ["OK","OK","OK","OK","OK","OK","OK","OK","OK","OK","OK","OK"]
			},
			{
				"accountNumber": "LOAN-789012",
				"creditor": "Auto Finance LLC",
				"type": "autoLoan",
				"status": "open",
				"openDate": "2022-02-10",
				"originalAmount": 28000,
				"balance": 18500,
				"monthlyPayment": 485,
				"paymentHistory": ["OK","OK","OK","OK","OK","OK","OK","OK","OK","OK","OK","OK"]
			},
			{
				"accountNumber": "MTG-555666777",
				"creditor": "Home Mortgage Corp",
				"type": "mortgage",
				"status": "open",
				"openDate": "2020-03-15",
				"originalAmount": 320000,
				"balance": 295000,
				"monthlyPayment": 1850,
				"paymentHistory": ["OK","OK","OK","OK","OK","OK","OK","OK","OK","OK","OK","OK"]
			}
		],
		"inquiries": [
			{"date": "2024-01-10", "creditor": "Capital One", "type": "hard"},
			{"date": "2023-11-05", "creditor": "Chase Bank", "type": "hard"},
			{"date": "2023-09-20", "creditor": "American Express", "type": "soft"}
		],
		"publicRecords": [],
		"collections": [],
		"metadata": {
			"version": "2.1.0",
			"provider": "TestProvider",
			"requestId": "REQ-2024-ABCDEF123456",
			"processingTimeMs": 245
		}
	}`)

	query := `
		INSERT INTO credit_data_order_v2 (id, organization_id, order_id, account_review, full_report, raw_report, created_at, updated_at)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
	`

	var completed int64
	var wg sync.WaitGroup
	jobs := make(chan int, *count)

	// Start workers
	for w := 0; w < *workers; w++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for range jobs {
				id := uuid.New().String()
				orderID := uuid.New().String()
				now := time.Now()

				tx, err := pool.Begin(ctx)
				if err != nil {
					log.Printf("Insert failed to begin tx: %v", err)
					atomic.AddInt64(&completed, 1)
					continue
				}

				// NOTE: The original repro issued `SET CIPHERSTASH.KEYSET_NAME`
				// here to mirror repo.setKeysetNameOnTx. For the passthrough
				// (no-encryption) memory test that SET is omitted: this proxy has
				// a default keyset configured, which rejects the SET, and it is
				// irrelevant to the per-statement type-check churn we measure.

				_, err = tx.Exec(ctx, query,
					id,
					orgID,
					orderID,
					false,
					testJSON,
					testJSON,
					now,
					now,
				)
				if err != nil {
					tx.Rollback(ctx)
					log.Printf("Insert failed: %v", err)
					atomic.AddInt64(&completed, 1)
					continue
				}

				if err := tx.Commit(ctx); err != nil {
					log.Printf("Insert failed to commit: %v", err)
					atomic.AddInt64(&completed, 1)
					continue
				}

				c := atomic.AddInt64(&completed, 1)
				if c%100 == 0 || c == int64(*count) {
					log.Printf("Inserted %d/%d records", c, *count)
				}
			}
		}()
	}

	// Send jobs
	for i := 0; i < *count; i++ {
		jobs <- i
	}
	close(jobs)

	// Wait for all workers to finish
	wg.Wait()

	fmt.Println("Done!")
}
