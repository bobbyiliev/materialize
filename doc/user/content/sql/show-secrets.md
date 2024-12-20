---
title: "SHOW SECRETS"
description: "`SHOW SECRETS` lists the names of the secrets securely stored in Materialize's secret management system."
menu:
  main:
    parent: commands
---

`SHOW SECRETS` lists the names of the secrets securely stored in Materialize's
secret management system. There is no way to show the contents of an existing
secret, though you can override it using the [`ALTER SECRET`](../alter-secret)
statement.

## Syntax

{{< diagram "show-secrets.svg" >}}

Field                | Use
---------------------|-----
_schema&lowbar;name_ | The schema to show secrets from. If omitted, secrets from the first schema in the search path are shown. For available schemas, see [`SHOW SCHEMAS`](../show-schemas).

## Examples

```mzsql
SHOW SECRETS;
```

```nofmt
         name
-----------------------
 kafka_ca_cert
 kafka_sasl_password
 kafka_sasl_username
```

```mzsql
SHOW SECRETS FROM public LIKE '%cert%';
```

```nofmt
         name
-----------------------
 kafka_ca_cert
```

## Related pages

- [`CREATE SECRET`](../create-secret)
- [`ALTER SECRET`](../alter-secret)
- [`DROP SECRET`](../drop-secret)
