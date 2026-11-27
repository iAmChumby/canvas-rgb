# Move group

Method and path: POST /api/v1/scenes/{id}/groups/{group}/move.

Area: Scenes and chains.

## Request

**POST /api/v1/scenes/{id}/groups/{group}/move**

Moves or copies one group between two scenes in one save.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{"to":"scene:other","copy":true,"expected_rev_from":4,"expected_rev_to":1}' "http://127.0.0.1:9430/api/v1/scenes/<id>/groups/<group>/move"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/scenes/<id>/groups/<group>/move", {
  method: "POST",
  headers: {
    Authorization: `Bearer ${token}`,
    "Content-Type": "application/json",
  },
  body: JSON.stringify({"to":"scene:other","copy":true,"expected_rev_from":4,"expected_rev_to":1}),
});
const data = await response.json();
```

## Types

**POST**

**Request body**: `GroupMove`.

| Field | Type | What it holds |
| --- | --- | --- |
| `to` | `string` | The id of the scene the group goes to. |
| `copy` | `boolean` | True copies the group: the source scene keeps the group and its revision, and the copy gets a fresh group id. |
| `expected_rev_from` | `number` | The revision of the source scene the request was made from. |
| `expected_rev_to` | `number` | The revision of the target scene the request was made from. |
| `dry_run` | `boolean` | True runs every check and saves nothing; the answer says saved false. Optional. |

**Answer body**: `GroupMoved`.

| Field | Type | What it holds |
| --- | --- | --- |
| `group_id` | `string` | The id the group has in the target scene: its own when the target lacks it, else a fresh one; a copy always has a fresh one. |
| `saved` | `boolean` | False for a dry run: nothing was written and the scenes below are what would be. |
| `scenes` | `array of Scene` | The scenes after the move, the source and then the target. A copy leaves the source scene as it was. |
| `before` | `array of Scene` | The scenes before the move, the source and then the target: what an undo writes back. |
| `taken` | `array of GroupTaken` | The lights taken out of the target's other groups. |
| `emptied` | `array of string` | The names of the groups that held no light afterwards and were removed. |

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the scene that holds the group. |
| `group` | `string` | The id of a group in the scene named by the `id` path parameter. Any scene, active or not. |

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

**POST**

Moves or copies one group between two scenes in one save.

```json
{
  "to": "scene:other",
  "copy": true,
  "expected_rev_from": 4,
  "expected_rev_to": 1
}
```
