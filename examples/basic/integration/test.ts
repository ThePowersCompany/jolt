import type { TypedTestResponse } from "generated-types";

declare const process: {
  env: Record<string, string | undefined>;
  exitCode?: number;
};

const baseUrl = process.env.JOLTR_BASIC_BASE_URL ?? "http://127.0.0.1:3000";
const endpointUrl = new URL("/api/test/typed", baseUrl);

main().catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});

async function main(): Promise<void> {
  const response = await fetch(endpointUrl);

  if (!response.ok) {
    throw new Error(`expected 2xx response from ${endpointUrl}, got ${response.status}`);
  }

  const body: unknown = await response.json();

  assertTypedTestResponse(body);
  assertContract(body);

  console.log(
    `verified TypedTestResponse v${body.contract_version} from ${endpointUrl}`,
  );
}

function assertTypedTestResponse(
  value: unknown,
): asserts value is TypedTestResponse {
  if (!isRecord(value)) {
    throw new Error("typed test response must be an object");
  }

  if (typeof value.contract_version !== "number") {
    throw new Error("typed test response contract_version must be a number");
  }

  if (typeof value.service !== "string") {
    throw new Error("typed test response service must be a string");
  }

  if (typeof value.ok !== "boolean") {
    throw new Error("typed test response ok must be a boolean");
  }

  if (
    !Array.isArray(value.features) ||
    !value.features.every((feature) => typeof feature === "string")
  ) {
    throw new Error("typed test response features must be string[]");
  }
}

function assertContract(body: TypedTestResponse): void {
  if (body.contract_version !== 1) {
    throw new Error(`expected contract_version 1, got ${body.contract_version}`);
  }

  if (body.service !== "joltr-basic-example") {
    throw new Error(`expected joltr-basic-example service, got ${body.service}`);
  }

  if (!body.ok) {
    throw new Error("expected ok to be true");
  }

  const expectedFeatures = ["endpoint-macro", "ts-export"];
  for (const feature of expectedFeatures) {
    if (!body.features.includes(feature)) {
      throw new Error(`expected feature ${feature} in ${body.features.join(",")}`);
    }
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
