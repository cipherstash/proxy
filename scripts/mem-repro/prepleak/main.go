// Tests the one unbounded code path identified in the proxy: named prepared
// statements are only removed from the per-connection statements map on an
// explicit Close/Deallocate. A client that prepares many uniquely-named
// statements on a single long-lived connection (and never deallocates) makes
// that map grow without bound.
//
// This uses a SINGLE connection and prepares -count uniquely-named statements.
package main

import (
	"context"
	"flag"
	"fmt"
	"log"

	"github.com/jackc/pgx/v5"
)

func main() {
	dsn := flag.String("dsn", "postgres://cipherstash:password@proxy:6432/cipherstash?sslmode=disable", "DSN")
	count := flag.Int("count", 100000, "number of uniquely-named prepared statements")
	flag.Parse()

	ctx := context.Background()
	conn, err := pgx.Connect(ctx, *dsn)
	if err != nil {
		log.Fatal("connect:", err)
	}
	defer conn.Close(ctx)

	for i := 0; i < *count; i++ {
		name := fmt.Sprintf("stmt_%d", i)
		// Unique name, same SQL. Never deallocated -> proxy retains one entry per name.
		_, err := conn.Prepare(ctx, name, "SELECT id, full_report FROM credit_data_order_v2 WHERE id = $1")
		if err != nil {
			log.Fatalf("prepare %d: %v", i, err)
		}
		if (i+1)%10000 == 0 {
			log.Printf("prepared %d/%d", i+1, *count)
		}
	}
	log.Printf("Done: %d uniquely-named prepared statements on one connection", *count)
}
