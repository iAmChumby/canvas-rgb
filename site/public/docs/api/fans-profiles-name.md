# Fan profiles

Method and path: PUT / DELETE /api/v1/fans/profiles/{name}.

Area: Fans.

## Request

**PUT /api/v1/fans/profiles/{name}**

Creates or replaces a named fan profile. A name in the body that differs from the path, or a value below the configured fan floor, answers 422.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"name":"Default","headers":{}}' "http://127.0.0.1:9430/api/v1/fans/profiles/<name>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/fans/profiles/<name>", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"name":"Default","headers":{}}),
});
const data = await response.json();
```

**DELETE /api/v1/fans/profiles/{name}**

Deletes a named fan profile.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/fans/profiles/<name>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/fans/profiles/<name>", {
  method: "DELETE",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `FanProfile`.

| Field | Type | What it holds |
| --- | --- | --- |
| `name` | `string` | The name of the fan profile. |
| `headers` | `an object keyed by text, with FanMode values` | The mode of each fan header, by header id. Headers that are not listed stay on BIOS. |

**Answer body**: `FanProfile`.

| Field | Type | What it holds |
| --- | --- | --- |
| `name` | `string` | The name of the fan profile. |
| `headers` | `an object keyed by text, with FanMode values` | The mode of each fan header, by header id. Headers that are not listed stay on BIOS. |

**DELETE**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `JSON object`.

The fields the daemon sends: the profile that was deleted.

| Field | Type | What it holds |
| --- | --- | --- |
| `ok` | `boolean` | True when the profile was deleted. |
| `name` | `string` | The profile that was deleted. |

| Path | Type | What it holds |
| --- | --- | --- |
| `name` | `string` | The name of the fan profile to change. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/fans` | none | FansStatus |
| PATCH `/api/v1/fans/headers/{h}` | FanMode | JSON object |
| PUT `/api/v1/fans/headers/{h}/availability` | PutFanHeaderAvailability | JSON object |
| PUT `/api/v1/fans/hot` | HotLimits | FansStatus |
| GET `/api/v1/fans/profile` | none | FanProfileName |
| PATCH `/api/v1/fans/profile` | FanProfileName | FanProfileName |
| PUT `/api/v1/fans/profiles/{name}` | FanProfile | FanProfile |
| DELETE `/api/v1/fans/profiles/{name}` | none | JSON object |

## Example

**PUT**

Creates or replaces a named fan profile. A name in the body that differs from the path, or a value below the configured fan floor, answers 422.

```json
{
  "name": "Default",
  "headers": {}
}
```

**DELETE**

Deletes a named fan profile.

This route takes no request body.
