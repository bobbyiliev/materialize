# Copyright Materialize, Inc. and contributors. All rights reserved.
#
# Use of this software is governed by the Business Source License
# included in the LICENSE file at the root of this repository.
#
# As of the Change Date specified in that file, in accordance with
# the Business Source License, use of this software will be governed
# by the Apache License, Version 2.0.

$ skip-if
SELECT mz_version_num() < 8300;

> SELECT name FROM mz_roles;
anonymous_http_user
group
joe
mz_analytics
mz_monitor
mz_monitor_redacted
mz_system
mz_support
materialize
superuser_login
"space role"

$ postgres-execute connection=postgres://superuser_login:some_bogus_password@${testdrive.materialize-sql-addr}
SELECT 1;

> SELECT name FROM mz_roles WHERE name = 'joe' OR name = 'group';
joe
group

> SELECT role.name AS role, member.name AS member, grantor.name AS grantor FROM mz_role_members membership LEFT JOIN mz_roles role ON membership.role_id = role.id LEFT JOIN mz_roles member ON membership.member = member.id LEFT JOIN mz_roles grantor ON membership.grantor = grantor.id;
group  joe  mz_system

$ postgres-execute connection=postgres://mz_system@${testdrive.materialize-internal-sql-addr}/materialize
DROP ROLE superuser_login;
DROP ROLE "space role";
DROP ROLE joe;
DROP ROLE group;
