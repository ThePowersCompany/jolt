import assert from "node:assert";
import { Spec } from "../server/types.generated";

export const host = `http://127.0.0.1:${process.env["SERVER_PORT"] ?? "3333"}`;

interface JsonResponse<T> extends Response {
  json(): Promise<T>;
}

export async function ping(): Promise<void> {
  const url = `${host}/ping`;
  const result = await fetch(url, {
    method: "GET",
  });
  assert(result.ok);
}

type Body<T> = T extends { body?: infer B } ? Exclude<B, undefined> : never;
type QueryParams<T> = T extends { queryParams?: infer Q }
  ? Exclude<Q, undefined>
  : never;

type IsNever<T> = [T] extends [never] ? true : false;

type BodyOpt<T> = IsNever<Body<T>> extends true ? {} : { body: Body<T> };

type QueryParamsOpt<T> =
  IsNever<QueryParams<T>> extends true ? {} : { queryParams: QueryParams<T> };

type Opts<T> = BodyOpt<T> & QueryParamsOpt<T>;

type RawQuery = `?${string}`;

type EndpointArgs<M extends keyof Spec, P extends keyof Spec[M] & string> = [
  endpoint: P | `${P}?${string}`,
  ...opts: {} extends BodyOpt<Spec[M][P]>
    ? [opts?: Opts<Spec[M][P]> | RawQuery]
    : [opts: Opts<Spec[M][P]> | RawQuery],
];

type EndpointResponse<
  M extends keyof Spec,
  P extends keyof Spec[M] & string,
> = Spec[M][P] extends { response: infer R } ? R : never;

type RequestArgs = [
  endpoint: string,
  opts?: RawQuery | { queryParams?: Record<string, any>; body?: unknown },
];

async function request<M extends keyof Spec, P extends keyof Spec[M] & string>(
  method: M,
  ...[endpoint, opts]: RequestArgs
): Promise<JsonResponse<EndpointResponse<M, P>>> {
  const queryParams = typeof opts === "string" ? opts : opts?.queryParams;
  const body = typeof opts === "string" ? undefined : opts?.body;

  let queryString = "";
  if (typeof queryParams === "string") {
    queryString = queryParams;
  } else if (queryParams != null) {
    queryString = `?${new URLSearchParams(queryParams)}`;
  }

  return fetch(`${host}${endpoint}${queryString}`, {
    method: method as string,
    headers: {
      ...(body !== undefined && { "Content-Type": "application/json" }),
    },
    ...(body !== undefined && { body: JSON.stringify(body) }),
  }) as Promise<JsonResponse<EndpointResponse<M, P>>>;
}

export const GET = <P extends keyof Spec["GET"] & string>(
  ...args: EndpointArgs<"GET", P>
) => request<"GET", P>("GET", ...args);

export const POST = <P extends keyof Spec["POST"] & string>(
  ...args: EndpointArgs<"POST", P>
) => request<"POST", P>("POST", ...args);

export const PUT = <P extends keyof Spec["PUT"] & string>(
  ...args: EndpointArgs<"PUT", P>
) => request<"PUT", P>("PUT", ...args);

export const PATCH = <P extends keyof Spec["PATCH"] & string>(
  ...args: EndpointArgs<"PATCH", P>
) => request<"PATCH", P>("PATCH", ...args);

export const DELETE = <P extends keyof Spec["DELETE"] & string>(
  ...args: EndpointArgs<"DELETE", P>
) => request<"DELETE", P>("DELETE", ...args);
