SELECT DISTINCT p.proname AS name FROM pg_proc p JOIN pg_aggregate a ON a.aggfnoid = p.oid;
