import test from "node:test";
import assert from "node:assert/strict";

import {
  apiFetch,
  apiHeaders,
  apiMaybeFetch,
  createApiClient,
  responseJson,
  responseJsonOrNull,
} from "./api_client.js";

function jsonResponse({
  ok = true,
  status = 200,
  statusText = "OK",
  body = {},
  jsonError = null,
} = {}) {
  return {
    ok,
    status,
    statusText,
    json: async () => {
      if (jsonError) {
        throw jsonError;
      }
      return body;
    },
  };
}

test("apiHeaders preserves extra headers and applies bearer token when present", () => {
  const extra = {
    Accept: "application/json",
    Authorization: "Basic old",
  };

  const withToken = apiHeaders(extra, "secret");
  assert.deepEqual(withToken, {
    Accept: "application/json",
    Authorization: "Bearer secret",
  });
  assert.notEqual(withToken, extra);

  assert.deepEqual(apiHeaders(extra, ""), extra);
});

test("createApiClient apiFetch adds late-bound auth headers and preserves init fields", async () => {
  const calls = [];
  let token = "first";
  const client = createApiClient({
    getToken: () => token,
    fetchImpl: async (path, init) => {
      calls.push([path, init]);
      return jsonResponse();
    },
  });

  await client.apiFetch("/v1/sessions", {
    method: "POST",
    headers: { Accept: "application/json" },
    body: "{}",
  });
  token = "second";
  await client.apiFetch("/health");

  assert.deepEqual(calls, [
    [
      "/v1/sessions",
      {
        method: "POST",
        headers: {
          Accept: "application/json",
          Authorization: "Bearer first",
        },
        body: "{}",
      },
    ],
    [
      "/health",
      {
        headers: {
          Authorization: "Bearer second",
        },
      },
    ],
  ]);
});

test("apiFetch reports JSON message, code, or HTTP fallback and preserves status", async () => {
  await assert.rejects(
    apiFetch("/conflict", {}, {
      fetchImpl: async () => jsonResponse({
        ok: false,
        status: 409,
        statusText: "Conflict",
        body: { message: "custom message" },
      }),
    }),
    (error) => {
      assert.equal(error.message, "custom message");
      assert.equal(error.status, 409);
      return true;
    },
  );

  await assert.rejects(
    apiFetch("/coded", {}, {
      fetchImpl: async () => jsonResponse({
        ok: false,
        status: 422,
        statusText: "Unprocessable Entity",
        body: { code: "BAD_INPUT" },
      }),
    }),
    (error) => {
      assert.equal(error.message, "BAD_INPUT");
      assert.equal(error.status, 422);
      return true;
    },
  );

  await assert.rejects(
    apiFetch("/broken-json", {}, {
      fetchImpl: async () => jsonResponse({
        ok: false,
        status: 500,
        statusText: "Internal Server Error",
        jsonError: new Error("not json"),
      }),
    }),
    (error) => {
      assert.equal(error.message, "500 Internal Server Error");
      assert.equal(error.status, 500);
      return true;
    },
  );
});

test("apiMaybeFetch maps 404 to null and leaves other failures intact", async () => {
  const missing = await apiMaybeFetch("/missing", {}, {
    fetchImpl: async () => jsonResponse({
      ok: false,
      status: 404,
      statusText: "Not Found",
      body: { message: "missing" },
    }),
  });
  assert.equal(missing, null);

  await assert.rejects(
    apiMaybeFetch("/denied", {}, {
      fetchImpl: async () => jsonResponse({
        ok: false,
        status: 403,
        statusText: "Forbidden",
        body: { message: "denied" },
      }),
    }),
    (error) => {
      assert.equal(error.message, "denied");
      assert.equal(error.status, 403);
      return true;
    },
  );
});

test("responseJsonOrNull returns null for empty responses and parses present responses", async () => {
  assert.equal(await responseJsonOrNull(null), null);
  assert.deepEqual(
    await responseJsonOrNull(jsonResponse({ body: { ok: true } })),
    { ok: true },
  );
});

test("responseJson applies a normalizer after parsing JSON", async () => {
  const calls = [];
  const result = await responseJson(
    jsonResponse({ body: { count: "7", label: "sessions" } }),
    (payload) => {
      calls.push(payload);
      return { ...payload, count: Number(payload.count) };
    },
  );

  assert.deepEqual(result, { count: 7, label: "sessions" });
  assert.deepEqual(calls, [{ count: "7", label: "sessions" }]);
});

test("responseJsonOrNull applies normalizers only for present responses", async () => {
  let normalizeCalls = 0;
  const normalize = (payload) => {
    normalizeCalls += 1;
    return { ok: Boolean(payload?.ok), normalized: true };
  };

  assert.equal(await responseJsonOrNull(null, normalize), null);
  assert.deepEqual(
    await responseJsonOrNull(jsonResponse({ body: { ok: 1 } }), normalize),
    { ok: true, normalized: true },
  );
  assert.equal(normalizeCalls, 1);
});

test("createApiClient exposes typed JSON response helpers", async () => {
  const client = createApiClient();

  assert.deepEqual(
    await client.responseJson(
      jsonResponse({ body: { value: "42" } }),
      (payload) => ({ value: Number(payload.value) }),
    ),
    { value: 42 },
  );
  assert.deepEqual(
    await client.responseJsonOrNull(
      jsonResponse({ body: { enabled: 1 } }),
      (payload) => ({ enabled: Boolean(payload.enabled) }),
    ),
    { enabled: true },
  );
});
