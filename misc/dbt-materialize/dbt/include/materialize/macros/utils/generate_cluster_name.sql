-- Copyright Materialize, Inc. and contributors. All rights reserved.
--
-- Licensed under the Apache License, Version 2.0 (the "License");
-- you may not use this file except in compliance with the License.
-- You may obtain a copy of the License in the LICENSE file at the
-- root of this repository, or online at
--
--     http://www.apache.org/licenses/LICENSE-2.0
--
-- Unless required by applicable law or agreed to in writing, software
-- distributed under the License is distributed on an "AS IS" BASIS,
-- WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
-- See the License for the specific language governing permissions and
-- limitations under the License.

{% macro generate_cluster_name_internal(custom_cluster_name, force_deploy_suffix=False) -%}

    {%- set cluster_name = adapter.dispatch('generate_cluster_name', 'materialize')(custom_cluster_name) -%}
    {%- set deploy_suffix = "_dbt_deploy" if var('deploy', False) or force_deploy_suffix else "" -%}
    {{ cluster_name }}{{ deploy_suffix }}

{%- endmacro %}

{% macro materialize__generate_cluster_name(custom_cluster_name) -%}

    {{ custom_cluster_name }}

{%- endmacro %}
