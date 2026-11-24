import createClient, { type Middleware } from "openapi-fetch";
import type { paths } from "./schema";

let csrfToken: string | null = null;

export function setCsrfToken(token: string | null) {
  csrfToken = token;
}

export function getCsrfToken(): string | null {
  return csrfToken;
}

/**
 * CSRF Header & Cookie Middleware for openapi-fetch
 */
const csrfMiddleware: Middleware = {
  async onRequest({ request }) {
    const isMutating = ["POST", "PUT", "PATCH", "DELETE"].includes(request.method.toUpperCase());
    if (isMutating && csrfToken) {
      request.headers.set("X-CSRF-Token", csrfToken);
    }
    return request;
  },
};

const defaultBaseUrl =
  typeof window !== "undefined" && window.location && window.location.origin && window.location.origin !== "null"
    ? window.location.origin
    : "http://localhost";

/**
 * Typed OpenAPI Fetch Client
 */
export const api = createClient<paths>({
  baseUrl: defaultBaseUrl,
  credentials: "include",
  fetch: (req: Request) => fetch(req),
});

api.use(csrfMiddleware);

export interface ApiFetchOptions extends RequestInit {
  params?: Record<string, string | number | boolean | undefined>;
}

export async function apiFetch<T>(path: string, options: ApiFetchOptions = {}): Promise<T> {
  const { params, headers = {}, ...rest } = options;

  let url = path;
  if (params) {
    const searchParams = new URLSearchParams();
    for (const [key, value] of Object.entries(params)) {
      if (value !== undefined) {
        searchParams.append(key, String(value));
      }
    }
    const queryString = searchParams.toString();
    if (queryString) {
      url += (url.includes("?") ? "&" : "?") + queryString;
    }
  }

  const reqHeaders: Record<string, string> = {
    Accept: "application/json",
    ...(headers as Record<string, string>),
  };

  const isMutating = ["POST", "PUT", "PATCH", "DELETE"].includes((rest.method || "GET").toUpperCase());
  if (isMutating && csrfToken) {
    reqHeaders["X-CSRF-Token"] = csrfToken;
  }

  if (rest.body && typeof rest.body === "string" && !reqHeaders["Content-Type"]) {
    reqHeaders["Content-Type"] = "application/json";
  }

  const response = await fetch(url, {
    ...rest,
    headers: reqHeaders,
    credentials: "include",
  });

  if (!response.ok) {
    let errorBody: any;
    try {
      errorBody = await response.json();
    } catch {
      errorBody = { message: response.statusText };
    }
    // The backend answers with RFC 9457 problem details: `detail` says what went
    // wrong in words, `code` is the stable machine code, `errors` the field errors.
    const err = new Error(errorBody.detail || errorBody.message || errorBody.title || "API request failed");
    (err as any).status = response.status;
    (err as any).code = errorBody.code;
    (err as any).errors = errorBody.errors;
    (err as any).details = errorBody;
    throw err;
  }

  // 202 and 204 answers usually carry no body.
  const text = await response.text();
  if (text.trim() === "") {
    return {} as T;
  }
  return JSON.parse(text) as T;
}
