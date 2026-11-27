# Scenes

Method and path: GET / POST / PUT /api/v1/scenes.

Area: Scenes and chains.

## Request

**GET /api/v1/scenes**

Lists the scenes and marks the active one.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/scenes"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/scenes", {
  method: "GET",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

**POST /api/v1/scenes**

Creates a scene from the groups in the request.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"name":"Gaming"}' "http://127.0.0.1:9430/api/v1/scenes"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/scenes", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"name":"Gaming"}),
});
const data = await response.json();
```

**PUT /api/v1/scenes**

Writes several scenes or their order in one save.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{}' "http://127.0.0.1:9430/api/v1/scenes"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/scenes", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({}),
});
const data = await response.json();
```

## Types

**GET**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `ScenesResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `active` | `string` | The id of the active scene. |
| `scenes` | `array of Scene` | The saved scenes. |
| `group_states` | `array of GroupState` | The active scene's groups, in its order. |

**POST**

**Request body**: `SceneCreate`.

| Field | Type | What it holds |
| --- | --- | --- |
| `name` | `string` | The name of the new scene. |
| `copy_of` | `string` | The id of a scene to copy the groups from. Optional. |
| `groups` | `array of NewGroup` | The groups the new scene holds, in order. The daemon mints each group's id and runs the scene rules before it saves. Optional. |
| `dry_run` | `boolean` | True runs every check and saves nothing; the answer says saved false. Optional. |

**Answer body**: `SceneAnswer`.

A saved scene object, or a dry-run answer with saved set to false.

**PUT**

**Request body**: `ScenesWrite`.

| Field | Type | What it holds |
| --- | --- | --- |
| `scenes` | `array of SceneWrite` | The scenes to replace whole. Leave them out for an order-only write. Optional. |
| `order` | `array of string` | The new order: every scene id exactly once. The same order as now answers 200 with no write. Optional. |
| `expected_order` | `array of string` | The order the caller last saw; a different current order answers 409 carrying it. Optional. |

**Answer body**: `ScenesResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `active` | `string` | The id of the active scene. |
| `scenes` | `array of Scene` | The saved scenes. |
| `group_states` | `array of GroupState` | The active scene's groups, in its order. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/chains` | none | ChainsResponse |
| POST `/api/v1/chains/{device}/{zone}/setup` | none | SetupStarted |
| DELETE `/api/v1/chains/{device}/{zone}/setup` | SetupEmpty | JSON object |
| POST `/api/v1/chains/{device}/{zone}/setup/commit` | SetupCommit | SetupCommitted |
| POST `/api/v1/chains/{device}/{zone}/setup/empty` | SetupEmpty | JSON object |
| POST `/api/v1/chains/{device}/{zone}/setup/light` | SetupLight | JSON object |
| GET `/api/v1/scenes` | none | ScenesResponse |
| POST `/api/v1/scenes` | SceneCreate | SceneAnswer |
| PUT `/api/v1/scenes` | ScenesWrite | ScenesResponse |
| PUT `/api/v1/scenes/{id}` | ScenePut | SceneAnswer |
| DELETE `/api/v1/scenes/{id}` | none | ScenesResponse |
| PUT `/api/v1/scenes/{id}/groups/{group}/brightness` | GroupBrightnessPut | Scene |
| POST `/api/v1/scenes/{id}/groups/{group}/move` | GroupMove | GroupMoved |
| PATCH `/api/v1/scenes/{id}/groups/{group}/props` | Props | Scene |
| PUT `/api/v1/scenes/active` | SceneActivePut | ScenesResponse |

## Example

**GET**

Lists the scenes and marks the active one.

This route takes no request body.

**POST**

Creates a scene from the groups in the request.

```json
{
  "name": "Gaming"
}
```

**PUT**

Writes several scenes or their order in one save.

```json
{}
```
