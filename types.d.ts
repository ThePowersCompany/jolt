// === DO NOT MODIFY ===
//
// Auto-generated type definitions
//
// === DO NOT MODIFY ===

export interface GetEndpoints {}

export interface PostEndpoints {
  "/example": {
    body: {
      is_active: boolean;
      first_name: string;
      last_name: string;
      company_id: number;
      site_id: number;
      role: string;
      email: string;
      username: string;
    };
    response: void;
  };
}

export interface PutEndpoints {}

export interface PatchEndpoints {}

export interface DeleteEndpoints {}
