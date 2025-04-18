# Copyright Materialize, Inc. and contributors. All rights reserved.
#
# Use of this software is governed by the Business Source License
# included in the LICENSE file at the root of this repository.
#
# As of the Change Date specified in that file, in accordance with
# the Business Source License, use of this software will be governed
# by the Apache License, Version 2.0.

#
# A cursory check for REPLICA IDENTITY DEFAULT. It is not a supported
# configuration but we should not panic.
#

$ postgres-execute connection=postgres://postgres:postgres@postgres
CREATE TABLE replica_identity_default (f1 INTEGER, f2 INTEGER, f3 INTEGER, PRIMARY KEY (f1));
ALTER TABLE replica_identity_default REPLICA IDENTITY DEFAULT;
INSERT INTO replica_identity_default VALUES (1,1,1), (2,2,2), (3,3,3), (4,4,4);

$ schema-registry-wait topic=postgres.public.replica_identity_default

> CREATE CONNECTION IF NOT EXISTS csr_conn TO CONFLUENT SCHEMA REGISTRY (
    URL '${testdrive.schema-registry-url}'
  );

> CREATE CONNECTION IF NOT EXISTS kafka_conn TO KAFKA (BROKER '${testdrive.kafka-addr}', SECURITY PROTOCOL PLAINTEXT);

> CREATE SOURCE replica_identity_default
  FROM KAFKA CONNECTION kafka_conn (TOPIC 'postgres.public.replica_identity_default');

> CREATE TABLE replica_identity_default_tbl FROM SOURCE replica_identity_default (REFERENCE "postgres.public.replica_identity_default")
  FORMAT AVRO USING CONFLUENT SCHEMA REGISTRY CONNECTION csr_conn
  ENVELOPE DEBEZIUM;

> SELECT * FROM replica_identity_default_tbl
1 1 1
2 2 2
3 3 3
4 4 4

$ postgres-execute connection=postgres://postgres:postgres@postgres
UPDATE replica_identity_default SET f2 = 5 WHERE f1 = 1;
UPDATE replica_identity_default SET f3 = 5 WHERE f1 = 2;
DELETE FROM replica_identity_default WHERE f1 = 3;
UPDATE replica_identity_default SET f1 = 5 WHERE f1 = 4;

# [btv] This succeeds now, due to upsert semantics.
# ! SELECT * FROM replica_identity_default_tbl;
# contains:Invalid data in source, saw retractions

> SELECT * FROM replica_identity_default_tbl
1 5 1
2 2 5
5 4 4
