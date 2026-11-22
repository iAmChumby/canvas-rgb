# Fan profile

Method and path: GET / PATCH /api/v1/fans/profile.

Area: Fans.

## Request

**GET /api/v1/fans/profile**

Returns the name of the active fan profile.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/fans/profile"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/fans/profile", {
  method: "GET",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

**PATCH /api/v1/fans/profile**

Changes the active fan profile.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PATCH -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"name":"Default"}' "http://127.0.0.1:9430/api/v1/fans/profile"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/fans/profile", {
  method: "PATCH",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"name":"Default"}),
});
const data = await response.json();
```

## Types

**GET**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `FanProfileName`.

| Field | Type | What it holds |
| --- | --- | --- |
| `name` | `string` | The name of the fan profile. |

**PATCH**

**Request body**: `FanProfileName`.

| Field | Type | What it holds |
| --- | --- | --- |
| `name` | `string` | The name of the fan profile. |

**Answer body**: `FanProfileName`.

| Field | Type | What it holds |
| --- | --- | --- |
| `name` | `string` | The name of the fan profile. |

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

**GET**

Returns the name of the active fan profile.

This route takes no request body.

**PATCH**

Changes the active fan profile.

```json
{
  "name": "Default"
}
```
