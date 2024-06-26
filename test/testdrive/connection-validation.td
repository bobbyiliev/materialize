# Copyright Materialize, Inc. and contributors. All rights reserved.
#
# Use of this software is governed by the Business Source License
# included in the LICENSE file at the root of this repository.
#
# As of the Change Date specified in that file, in accordance with
# the Business Source License, use of this software will be governed
# by the Apache License, Version 2.0.

$ skip-consistency-checks reason="workflow uses SSH keys which we currently can't check"

$ postgres-execute connection=postgres://mz_system:materialize@${testdrive.materialize-internal-sql-addr}
ALTER SYSTEM SET enable_connection_validation_syntax = true

> CREATE CONNECTION kafka_conn TO KAFKA (BROKER '${testdrive.kafka-addr}', SECURITY PROTOCOL PLAINTEXT)

> VALIDATE CONNECTION kafka_conn

> DROP CONNECTION kafka_conn

# Explicitly enable validation

> CREATE CONNECTION kafka_conn TO KAFKA (BROKER '${testdrive.kafka-addr}', SECURITY PROTOCOL PLAINTEXT) WITH (VALIDATE = true)

> VALIDATE CONNECTION kafka_conn

> DROP CONNECTION kafka_conn

# WITH (VALIDATE) variant

> CREATE CONNECTION kafka_conn TO KAFKA (BROKER '${testdrive.kafka-addr}', SECURITY PROTOCOL PLAINTEXT) WITH (VALIDATE)

> VALIDATE CONNECTION kafka_conn

> DROP CONNECTION kafka_conn

> CREATE CONNECTION invalid_tunnel TO SSH TUNNEL (HOST 'invalid', USER 'invalid', PORT 22)

! CREATE CONNECTION invalid_kafka_conn TO KAFKA (BROKERS ('${testdrive.kafka-addr}' USING SSH TUNNEL invalid_tunnel), SECURITY PROTOCOL PLAINTEXT)
contains:failed to lookup address information

# Create the connection without validation and validate later
> CREATE CONNECTION invalid_kafka_conn TO KAFKA (BROKERS ('${testdrive.kafka-addr}' USING SSH TUNNEL invalid_tunnel), SECURITY PROTOCOL PLAINTEXT) WITH (VALIDATE = false)

! VALIDATE CONNECTION invalid_kafka_conn
contains:failed to lookup address information
