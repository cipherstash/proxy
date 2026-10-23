-- This migration must run through Proxy so values assigned to EQL domains are
-- encrypted before PostgreSQL stores them.
TRUNCATE burnin_commerce_order_lines, burnin_commerce_orders,
    burnin_commerce_products, burnin_commerce_customers,
    burnin_type_lab_bulk_values, burnin_type_lab_samples;
