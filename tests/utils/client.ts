const BASE_URL = process.env["TEST_SERVER_URL"] ?? "http://localhost:3333";

type Body = Record<string, unknown> | unknown[] | null | undefined;

async function request(method: string, endpoint: string, body: Body): Promise<Response> {
  return await fetch(`${BASE_URL}${endpoint}`, {
    method: method,
    headers: {
      'Content-Type': 'application/json'
    },
    ...(body ? { body: JSON.stringify(body) } : {}),
  });
}

export function GET(endpoint: string, body?: Body): Promise<Response> {
  return request("GET", endpoint, body);
}

export function POST(endpoint: string, body?: Body): Promise<Response> {
  return request("POST", endpoint, body);
}

export function PUT(endpoint: string, body?: Body): Promise<Response> {
  return request("PUT", endpoint, body);
}

export function PATCH(endpoint: string, body?: Body): Promise<Response> {
  return request("PATCH", endpoint, body);
}

export function DELETE(endpoint: string, body?: Body): Promise<Response> {
  return request("DELETE", endpoint, body);
}

