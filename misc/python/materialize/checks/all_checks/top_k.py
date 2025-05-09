# Copyright Materialize, Inc. and contributors. All rights reserved.
#
# Use of this software is governed by the Business Source License
# included in the LICENSE file at the root of this repository.
#
# As of the Change Date specified in that file, in accordance with
# the Business Source License, use of this software will be governed
# by the Apache License, Version 2.0.
from textwrap import dedent

from materialize.checks.actions import Testdrive
from materialize.checks.checks import Check, externally_idempotent


def schema() -> str:
    return dedent(
        """
       $ set schema={
           "type" : "record",
           "name" : "test",
           "fields" : [
               {"name":"f1", "type":"string"}
           ]
         }
       """
    )


@externally_idempotent(False)
class BasicTopK(Check):
    def initialize(self) -> Testdrive:
        return Testdrive(
            dedent(
                """
            > CREATE TABLE basic_topk_table (f1 INTEGER);
            > INSERT INTO basic_topk_table VALUES (1), (2), (2), (3), (3), (3), (NULL), (NULL), (NULL), (NULL);
            """
            )
        )

    def manipulate(self) -> list[Testdrive]:
        return [
            Testdrive(dedent(s))
            for s in [
                """
                > INSERT INTO basic_topk_table SELECT * FROM basic_topk_table
                > CREATE MATERIALIZED VIEW basic_topk_view1 AS SELECT f1, COUNT(f1) FROM basic_topk_table GROUP BY f1 ORDER BY f1 DESC NULLS LAST LIMIT 2;
                > INSERT INTO basic_topk_table SELECT * FROM basic_topk_table;

                > CREATE VIEW view_with_limit_offset_1a AS SELECT DISTINCT f1 FROM basic_topk_table ORDER BY f1 DESC NULLS LAST LIMIT 2 OFFSET 1;

                # offset and limit reordered
                > CREATE VIEW view_with_limit_offset_1b AS SELECT DISTINCT f1 FROM basic_topk_table ORDER BY f1 DESC NULLS LAST OFFSET 1 LIMIT 2;
                    """,
                """
                > INSERT INTO basic_topk_table SELECT * FROM basic_topk_table;
                > CREATE MATERIALIZED VIEW basic_topk_view2 AS SELECT f1, COUNT(f1) FROM basic_topk_table GROUP BY f1 ORDER BY f1 ASC NULLS FIRST LIMIT 2;
                > INSERT INTO basic_topk_table SELECT * FROM basic_topk_table;

                > CREATE VIEW view_with_limit_offset_2a AS SELECT DISTINCT f1 FROM basic_topk_table ORDER BY f1 DESC NULLS LAST LIMIT 2 OFFSET 1;

                # offset and limit reordered
                > CREATE VIEW view_with_limit_offset_2b AS SELECT DISTINCT f1 FROM basic_topk_table ORDER BY f1 DESC NULLS LAST OFFSET 1 LIMIT 2;
                    """,
            ]
        ]

    def validate(self) -> Testdrive:
        return Testdrive(
            dedent(
                """
                > SELECT * FROM basic_topk_view1;
                2 32
                3 48

                > SELECT * FROM basic_topk_view2;
                1 16
                <null> 0

                > SELECT * FROM view_with_limit_offset_1a;
                2
                1

                > SELECT * FROM view_with_limit_offset_2a;
                2
                1

                > SELECT * FROM view_with_limit_offset_1b;
                2
                1

                > SELECT * FROM view_with_limit_offset_2b;
                2
                1
                """
            )
        )


@externally_idempotent(False)
class MonotonicTopK(Check):
    def initialize(self) -> Testdrive:
        return Testdrive(
            schema()
            + dedent(
                """
                $ kafka-create-topic topic=monotonic-topk

                $ kafka-ingest format=avro topic=monotonic-topk schema=${schema} repeat=1
                {"f1": "A"}

                > CREATE SOURCE monotonic_topk_source_src
                  FROM KAFKA CONNECTION kafka_conn (TOPIC 'testdrive-monotonic-topk-${testdrive.seed}')
                > CREATE TABLE monotonic_topk_source FROM SOURCE monotonic_topk_source_src (REFERENCE "testdrive-monotonic-topk-${testdrive.seed}")
                  FORMAT AVRO USING CONFLUENT SCHEMA REGISTRY CONNECTION csr_conn
                  ENVELOPE NONE
            """
            )
        )

    def manipulate(self) -> list[Testdrive]:
        return [
            Testdrive(schema() + dedent(s))
            for s in [
                """
                $ kafka-ingest format=avro topic=monotonic-topk schema=${schema} repeat=2
                {"f1": "B"}
                > CREATE MATERIALIZED VIEW monotonic_topk_view1 AS SELECT f1, COUNT(f1) FROM monotonic_topk_source GROUP BY f1 ORDER BY f1 DESC NULLS LAST LIMIT 2;
                $ kafka-ingest format=avro topic=monotonic-topk schema=${schema} repeat=3
                {"f1": "C"}
                """,
                """
                $ kafka-ingest format=avro topic=monotonic-topk schema=${schema} repeat=4
                {"f1": "D"}
                > CREATE MATERIALIZED VIEW monotonic_topk_view2 AS SELECT f1, COUNT(f1) FROM monotonic_topk_source GROUP BY f1 ORDER BY f1 ASC NULLS FIRST LIMIT 2;
                $ kafka-ingest format=avro topic=monotonic-topk schema=${schema} repeat=5
                {"f1": "E"}
                """,
            ]
        ]

    def validate(self) -> Testdrive:
        return Testdrive(
            dedent(
                """
                > SELECT * FROM monotonic_topk_view1;
                E 5
                D 4

                > SELECT * FROM monotonic_topk_view2;
                A 1
                B 2
                """
            )
        )


@externally_idempotent(False)
class MonotonicTop1(Check):
    def initialize(self) -> Testdrive:
        return Testdrive(
            schema()
            + dedent(
                """
                $ kafka-create-topic topic=monotonic-top1

                $ kafka-ingest format=avro topic=monotonic-top1 schema=${schema} repeat=1
                {"f1": "A"}

                > CREATE SOURCE monotonic_top1_source_src
                  FROM KAFKA CONNECTION kafka_conn (TOPIC 'testdrive-monotonic-top1-${testdrive.seed}')
                > CREATE TABLE monotonic_top1_source FROM SOURCE monotonic_top1_source_src (REFERENCE "testdrive-monotonic-top1-${testdrive.seed}")
                  FORMAT AVRO USING CONFLUENT SCHEMA REGISTRY CONNECTION csr_conn
                  ENVELOPE NONE
            """
            )
        )

    def manipulate(self) -> list[Testdrive]:
        return [
            Testdrive(schema() + dedent(s))
            for s in [
                """
                $ kafka-ingest format=avro topic=monotonic-top1 schema=${schema} repeat=2
                {"f1": "B"}
                > CREATE MATERIALIZED VIEW monotonic_top1_view1 AS SELECT f1, COUNT(f1) FROM monotonic_top1_source GROUP BY f1 ORDER BY f1 DESC NULLS LAST LIMIT 1;
                $ kafka-ingest format=avro topic=monotonic-top1 schema=${schema} repeat=3
                {"f1": "C"}
                """,
                """
                $ kafka-ingest format=avro topic=monotonic-top1 schema=${schema} repeat=4
                {"f1": "C"}
                > CREATE MATERIALIZED VIEW monotonic_top1_view2 AS SELECT f1, COUNT(f1) FROM monotonic_top1_source GROUP BY f1 ORDER BY f1 ASC NULLS FIRST LIMIT 1;
                $ kafka-ingest format=avro topic=monotonic-top1 schema=${schema} repeat=5
                {"f1": "D"}
                """,
            ]
        ]

    def validate(self) -> Testdrive:
        return Testdrive(
            dedent(
                """
                > SELECT * FROM monotonic_top1_view1;
                D 5

                > SELECT * FROM monotonic_top1_view2;
                A 1
                """
            )
        )
