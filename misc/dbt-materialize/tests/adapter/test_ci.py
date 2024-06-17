# Copyright Materialize, Inc. and contributors. All rights reserved.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License in the LICENSE file at the
# root of this repository, or online at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

import pytest
from dbt.tests.util import run_dbt


class TestClusterOps:

    @pytest.fixture(autouse=True)
    def cleanup(self, project):
        project.run_sql("DROP CLUSTER IF EXISTS test_cluster CASCADE")
        project.run_sql("DROP CLUSTER IF EXISTS test_cluster_dbt_deploy CASCADE")

    def test_create_and_drop_cluster(self, project):
        # Test creating a cluster
        run_dbt(
            [
                "run-operation",
                "create_cluster",
                "--args",
                '{"cluster_name": "test_cluster", "size": "1", "replication_factor": 1, "ignore_existing_objects": true, "force_deploy_suffix": true}',
            ]
        )

        # Verify cluster creation
        result = project.run_sql(
            "SELECT name FROM mz_clusters WHERE name = 'test_cluster_dbt_deploy'",
            fetch="one",
        )
        assert result is not None, "Cluster was not created successfully"

        # Test dropping the cluster
        run_dbt(
            [
                "run-operation",
                "drop_cluster",
                "--args",
                '{"cluster_name": "test_cluster_dbt_deploy"}',
            ]
        )

        # Verify cluster dropping
        result = project.run_sql(
            "SELECT name FROM mz_clusters WHERE name = 'test_cluster_dbt_deploy'",
            fetch="one",
        )
        assert result is None, "Cluster was not dropped successfully"

    def test_create_existing_cluster(self, project):
        # Create the cluster for the first time
        run_dbt(
            [
                "run-operation",
                "create_cluster",
                "--args",
                '{"cluster_name": "test_cluster", "size": "1", "replication_factor": 1, "ignore_existing_objects": true, "force_deploy_suffix": true}',
            ]
        )

        # Try creating the same cluster again
        run_dbt(
            [
                "run-operation",
                "create_cluster",
                "--args",
                '{"cluster_name": "test_cluster", "size": "1", "replication_factor": 1, "ignore_existing_objects": true, "force_deploy_suffix": true}',
            ]
        )

        # Verify the cluster exists
        result = project.run_sql(
            "SELECT name FROM mz_clusters WHERE name = 'test_cluster_dbt_deploy'",
            fetch="one",
        )
        assert (
            result is not None
        ), "Cluster should exist after attempting to create it again"

        # Cleanup
        run_dbt(
            [
                "run-operation",
                "drop_cluster",
                "--args",
                '{"cluster_name": "test_cluster_dbt_deploy"}',
            ]
        )

    def test_drop_nonexistent_cluster(self, project):
        # Attempt to drop a nonexistent cluster
        run_dbt(
            [
                "run-operation",
                "drop_cluster",
                "--args",
                '{"cluster_name": "nonexistent_cluster"}',
            ]
        )

        # Verify no cluster exists
        result = project.run_sql(
            "SELECT name FROM mz_clusters WHERE name = 'nonexistent_cluster'",
            fetch="one",
        )
        assert result is None, "Nonexistent cluster should not exist"

    def test_create_cluster_with_objects(self, project):
        # Create a cluster and add an object to it
        run_dbt(
            [
                "run-operation",
                "create_cluster",
                "--args",
                '{"cluster_name": "test_cluster", "size": "1", "replication_factor": 1, "ignore_existing_objects": true, "force_deploy_suffix": true}',
            ]
        )
        project.run_sql("CREATE MATERIALIZED VIEW test_view AS SELECT 1")

        # Attempt to create the same cluster without ignoring existing objects
        with pytest.raises(Exception):
            run_dbt(
                [
                    "run-operation",
                    "create_cluster",
                    "--args",
                    '{"cluster_name": "test_cluster", "size": "1", "replication_factor": 1, "ignore_existing_objects": false, "force_deploy_suffix": true}',
                ],
                expect_pass=False,
            )

        # Cleanup
        project.run_sql("DROP MATERIALIZED VIEW IF EXISTS test_view")
        run_dbt(
            [
                "run-operation",
                "drop_cluster",
                "--args",
                '{"cluster_name": "test_cluster_dbt_deploy"}',
            ]
        )
