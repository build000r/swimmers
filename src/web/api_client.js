function defaultFetch(...args) {
  return globalThis.fetch(...args);
}

export function apiHeaders(extra = {}, token = "") {
  const headers = { ...extra };
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }
  return headers;
}

export async function apiFetch(path, init = {}, options = {}) {
  const {
    getToken = () => "",
    fetchImpl = defaultFetch,
  } = options;
  const headers = apiHeaders(init.headers ?? {}, getToken());
  const response = await fetchImpl(path, { ...init, headers });
  if (!response.ok) {
    let message = `${response.status} ${response.statusText}`;
    try {
      const json = await response.json();
      if (json?.message) {
        message = json.message;
      } else if (json?.code) {
        message = json.code;
      }
    } catch (_) {
      // Keep the HTTP fallback message.
    }
    const error = new Error(message);
    error.status = response.status;
    throw error;
  }
  return response;
}

export async function apiMaybeFetch(path, init = {}, options = {}) {
  try {
    return await apiFetch(path, init, options);
  } catch (error) {
    if (error?.status === 404) {
      return null;
    }
    throw error;
  }
}

export async function responseJsonOrNull(response) {
  if (!response) {
    return null;
  }
  return response.json();
}

export function createApiClient(options = {}) {
  return {
    apiHeaders: (extra = {}) => apiHeaders(extra, options.getToken?.() ?? ""),
    apiFetch: (path, init = {}) => apiFetch(path, init, options),
    apiMaybeFetch: (path, init = {}) => apiMaybeFetch(path, init, options),
    responseJsonOrNull,
  };
}
