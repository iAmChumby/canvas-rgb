# Scene

Method and path: PUT / DELETE /api/v1/scenes/{id}.

Area: Scenes and chains.

## Request

**PUT /api/v1/scenes/{id}**

Replaces one scene and its groups.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"scene":{"id":"everything","name":"Everything","rev":1,"groups":[{"id":"everything","name":"Everything","members":["openrgb:2a3b4c5d#z0"],"plays":{"type":"effect","effect":"solid","props":{}},"mode":"flow_across","brightness":100}]},"expected_rev":2}' "http://127.0.0.1:9430/api/v1/scenes/<id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/scenes/<id>", {
  method: "PUT",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"scene":{"id":"everything","name":"Everything","rev":1,"groups":[{"id":"everything","name":"Everything","members":["openrgb:2a3b4c5d#z0"],"plays":{"type":"effect","effect":"solid","props":{}},"mode":"flow_across","brightness":100}]},"expected_rev":2}),
});
const data = await response.json();
```

**DELETE /api/v1/scenes/{id}**

Deletes one scene.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/scenes/<id>"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/scenes/<id>", {
  method: "DELETE",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

## Types

**PUT**

**Request body**: `ScenePut`.

| Field | Type | What it holds |
| --- | --- | --- |
| `scene` | `Scene` | The scene to save. |
| `expected_rev` | `number` | The revision the change was made from. |
| `dry_run` | `boolean` | True runs every check and saves nothing; the answer says saved false. Optional. |

**Answer body**: `SceneAnswer`.

A saved scene object, or a dry-run answer with saved set to false.

**DELETE**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `ScenesResponse`.

| Field | Type | What it holds |
| --- | --- | --- |
| `active` | `string` | The id of the active scene. |
| `scenes` | `array of Scene` | The saved scenes. |
| `group_states` | `array of GroupState` | The active scene's groups, in its order. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the scene the route names. |

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

**PUT**

Replaces one scene and its groups.

```json
{
  "scene": {
    "id": "everything",
    "name": "Everything",
    "rev": 1,
    "groups": [
      {
        "id": "everything",
        "name": "Everything",
        "members": [
          "openrgb:2a3b4c5d#z0"
        ],
        "plays": {
          "type": "effect",
          "effect": "solid",
          "props": {}
        },
        "mode": "flow_across",
        "brightness": 100
      }
    ]
  },
  "expected_rev": 2
}
```

**DELETE**

Deletes one scene.

This route takes no request body.
