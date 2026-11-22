# Effect thumbnail

Method and path: GET /api/v1/effects/{id}/thumbnail.

Area: Effects.

## Request

**GET /api/v1/effects/{id}/thumbnail**

Returns the effect's thumbnail as bytes: three 16-bit little-endian counts (width, height, frame count) followed by raw RGB frames, rendered without touching the devices. The answer is sent as application/octet-stream.

```sh
# Check who owns port 9430 first; see the Verify the listener before you send the token section of the getting-started page
curl -X GET -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:9430/api/v1/effects/<id>/thumbnail"
```

```js
const response = await fetch("http://127.0.0.1:9430/api/v1/effects/<id>/thumbnail", {
  method: "GET",
  headers: {
    Authorization: `Bearer ${token}`,
  },
});
const data = await response.json();
```

## Types

**GET**

**Request body**: `none`.

No field list is published for this body.

**Answer body**: `Binary thumbnail`.

Binary bytes: three 16-bit little-endian counts (width, height, frame count) followed by raw RGB frames.

| Path | Type | What it holds |
| --- | --- | --- |
| `id` | `string` | The id of the effect whose tile is rendered. |

## Route index

| Method / path | Body | Answer |
| --- | --- | --- |
| GET `/api/v1/effects` | none | EffectsResponse |
| GET `/api/v1/effects/{id}` | none | EffectDetail |
| PUT `/api/v1/effects/{id}` | EffectDetail | JSON object |
| DELETE `/api/v1/effects/{id}` | none | DeleteEffectResponse |
| POST `/api/v1/effects/{id}/activate` | ActivateEffect | JSON object |
| PATCH `/api/v1/effects/{id}/defaults` | Props | JSON object |
| PUT `/api/v1/effects/{id}/favorite` | SetFavorite | JSON object |
| POST `/api/v1/effects/{id}/presets` | PresetCreate | PresetsResponse |
| PUT `/api/v1/effects/{id}/presets/{name}` | PresetReplace | PresetsResponse |
| DELETE `/api/v1/effects/{id}/presets/{name}` | none | PresetsResponse |
| GET `/api/v1/effects/{id}/source` | none | EffectSource |
| GET `/api/v1/effects/{id}/thumbnail` | none | Binary thumbnail |
| GET `/api/v1/effects/draft` | none | DraftStatus |
| PUT `/api/v1/effects/draft` | DraftRequest | DraftStatus |
| DELETE `/api/v1/effects/draft` | none | DraftStatus |
| POST `/api/v1/effects/import` | ImportEffect | ImportEffectResponse |
| POST `/api/v1/effects/preview` | EffectPreviewRequest | EffectPreviewResponse |

## Example

**GET**

Returns the effect's thumbnail as bytes: three 16-bit little-endian counts (width, height, frame count) followed by raw RGB frames, rendered without touching the devices. The answer is sent as application/octet-stream.

This route takes no request body.
