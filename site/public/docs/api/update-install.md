# Install update

Method and path: POST /api/v1/update/install.

Area: Settings and update.

## Request

**POST /api/v1/update/install**

Installs a signed update package.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"installer":"D:\\Downloads\\canvas-rgb-0.18.1-setup.exe","signature":"untrusted comment: minisign signature\nRWQf6X8s1m4n3p2q1r0s7t6u5v4w3x2y1z0a9b8c7d6e5f4g3h2j1k0l9m8n7p6q5r4s3t2u1"}' "http://127.0.0.1:9430/api/v1/update/install"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/update/install", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"installer":"D:\\Downloads\\canvas-rgb-0.18.1-setup.exe","signature":"untrusted comment: minisign signature\nRWQf6X8s1m4n3p2q1r0s7t6u5v4w3x2y1z0a9b8c7d6e5f4g3h2j1k0l9m8n7p6q5r4s3t2u1"}),
});
const data = await response.json();
```

## Types

**POST**

**Request body**: `InstallUpdateRequest`.

| Field | Type | What it holds |
| --- | --- | --- |
| `installer` | `string` | The absolute path of the installer, readable by the calling user. |
| `signature` | `string` | The text of the installer's signature file. |

**Answer body**: `InstallUpdateResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `version` | `string` | The signed version the update service started installing. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/config/export` | none | ExportBundle |
| POST `/api/v1/config/import` | ExportBundle | JSON object |
| POST `/api/v1/first-run/dismiss-zone-panel` | none | JSON object |
| POST `/api/v1/onboarding/complete` | none | JSON object |
| GET `/api/v1/settings` | none | Settings |
| PUT `/api/v1/settings` | Settings | Settings |
| POST `/api/v1/update/install` | InstallUpdateRequest | InstallUpdateResponse |

## Example

**POST**

Installs a signed update package.

```json
{
  "installer": "D:\\Downloads\\canvas-rgb-0.18.1-setup.exe",
  "signature": "untrusted comment: minisign signature\nRWQf6X8s1m4n3p2q1r0s7t6u5v4w3x2y1z0a9b8c7d6e5f4g3h2j1k0l9m8n7p6q5r4s3t2u1"
}
```
