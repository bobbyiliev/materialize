# Copyright Materialize, Inc. and contributors. All rights reserved.
#
# Use of this software is governed by the Business Source License
# included in the LICENSE file at the root of this repository.
#
# As of the Change Date specified in that file, in accordance with
# the Business Source License, use of this software will be governed
# by the Apache License, Version 2.0.

#
# Test DROP COLUMN on nullable columns
#

$ postgres-execute connection=postgres://postgres:postgres@postgres
CREATE TABLE alter_drop_column (f1 INTEGER PRIMARY KEy, col_null_no_default INTEGER, col_null_default INTEGER DEFAULT 999);
INSERT INTO alter_drop_column VALUES (123, 234, 345);

$ schema-registry-wait topic=postgres.public.alter_drop_column

> CREATE CONNECTION IF NOT EXISTS csr_conn TO CONFLUENT SCHEMA REGISTRY (
    URL '${testdrive.schema-registry-url}'
  );

> CREATE CONNECTION IF NOT EXISTS kafka_conn TO KAFKA (BROKER '${testdrive.kafka-addr}', SECURITY PROTOCOL PLAINTEXT);

> CREATE SOURCE alter_drop_column
  FROM KAFKA CONNECTION kafka_conn (TOPIC 'postgres.public.alter_drop_column');

> CREATE TABLE alter_drop_column_tbl FROM SOURCE alter_drop_column (REFERENCE "postgres.public.alter_drop_column")
  FORMAT AVRO USING CONFLUENT SCHEMA REGISTRY CONNECTION csr_conn
  ENVELOPE DEBEZIUM;

> SELECT * FROM alter_drop_column_tbl;
123 234 345

$ postgres-execute connection=postgres://postgres:postgres@postgres
ALTER TABLE alter_drop_column DROP COLUMN col_null_default;
INSERT INTO alter_drop_column VALUES (1234);

# Prior to Debezium 1.6, NULLs would be returned for col_null_default in new rows.
# Starting with Debezium 1.6, the 999 DEFAULT value will be returned instead.

> SELECT * FROM alter_drop_column_tbl;
123 234 345
1234 <null> 999
