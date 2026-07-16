// === DO NOT MODIFY ===
//
// Auto-generated type definitions
//
// === DO NOT MODIFY ===

type SetRequired<T, K extends keyof T> = Omit<T, K> & Required<Pick<T, K>>;

type AnyOf<T, K extends keyof T = keyof T> = {
  [P in K]: SetRequired<T, P>;
}[K];

type Without<T, U> = { [P in Exclude<keyof T, keyof U>]?: never };

type XOR<T, U> = T | U extends object
  ? (Without<T, U> & U) | (Without<U, T> & T)
  : T | U;

export type AnyOfQpDetails = {
  query: string | null;
  email: string | null;
  username: string | null;
  role: string | null;
};

export type GatedGroupQpDetails = {
  room: number;
  limit: number;
  cursor_present: boolean;
  cursor: number | null;
  before: boolean | null;
};

export type NestedOptionalQpDetails = {
  page: number;
  category: string | null;
  status: string | null;
};

export type UnionVariantQpDetails = {
  variant: string;
  id?: number | null;
  start_date?: number | null;
  line?: number | null;
  shift?: number | null;
};

export type Spec = {
  GET: {
    "/any-of": {
      queryParams: AnyOf<{
        query?: string;
        email?: string;
        username?: string;
        role?: string;
      }>;
      response: AnyOfQpDetails;
    };
    "/gated-group": {
      queryParams: XOR<
        {
          cursor: number;
          before?: boolean;
        },
        {}
      > & {
        room: number;
        limit?: number;
      };
      response: GatedGroupQpDetails;
    };
    "/nested-optional": {
      queryParams?: {
        page?: number;
        category?: string;
        status?: string;
      };
      response: NestedOptionalQpDetails;
    };
    "/union-variant": {
      queryParams?: XOR<
        { id: number },
        XOR<
          {
            start_date: number;
            end_date?: number;
            line?: number;
            shift?: number;
          },
          {
            line?: number;
            shift?: number;
          }
        >
      >;
      response: UnionVariantQpDetails;
    };
  };
  POST: {};
  PUT: {};
  PATCH: {};
  DELETE: {};
};
