import { readFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import { emitTypes } from "./reader-types/emitter.mjs"
import { checkOutputs, writeOutputs } from "./reader-types/outputs.mjs"

const scriptDirectory = dirname(fileURLToPath(import.meta.url))
const repositoryRoot = resolve(scriptDirectory, "../..")
const webRoot = resolve(repositoryRoot, "web")
const artifacts = [
  {
    source: "docs/openapi/ai-provider-v1.json",
    output: "src/features/ai/api/provider.generated.ts",
    aliases: {},
  },
  {
    source: "docs/openapi/ai-content-v1.json",
    output: "src/features/ai/api/content.generated.ts",
    aliases: {},
  },
  {
    source: "docs/openapi/preferences-v2.json",
    output: "src/features/preferences/api/preferences.generated.ts",
    aliases: {},
  },
  {
    source: "docs/openapi/profile-v2.json",
    output: "src/features/profile/api/profile.generated.ts",
    aliases: {},
  },
  {
    source: "docs/openapi/translation-v3.json",
    output: "src/features/translation/api/translation.generated.ts",
    aliases: {},
  },
  {
    source: "docs/openapi/organization-v1.json",
    output: "src/features/reader/api/organization.generated.ts",
    aliases: {
      CategoryResponse: "Category",
      CategoryListResponse: "CategoryList",
    },
  },
  {
    source: "docs/openapi/subscription-v1.json",
    output: "src/features/reader/api/subscription.generated.ts",
    aliases: {
      SubscriptionResponse: "Subscription",
      SubscriptionPageResponse: "SubscriptionPage",
      RefreshResponse: "Refresh",
      ApiErrorEnvelope: "ErrorEnvelope",
    },
  },
  {
    source: "docs/openapi/reader-v1.json",
    output: "src/features/reader/api/reader.generated.ts",
    aliases: {
      ApiErrorEnvelope: "ErrorEnvelope",
    },
  },
]

const options = parseArguments(process.argv.slice(2))
const outputs = artifacts.map(({ source, output, aliases }) => {
  const document = JSON.parse(readFileSync(resolve(repositoryRoot, source), "utf8"))
  return { path: output, content: emitTypes(document, source, aliases) }
})

if (options.check) {
  if (!checkOutputs(options.outputRoot, outputs)) process.exitCode = 1
} else {
  writeOutputs(options.outputRoot, outputs)
}

function parseArguments(args) {
  let check = false
  let outputRoot = webRoot
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index]
    if (argument === "--check") {
      check = true
      continue
    }
    if (argument === "--output-root") {
      const value = args[index + 1]
      if (!value) throw new Error("--output-root requires a path")
      outputRoot = resolve(value)
      index += 1
      continue
    }
    throw new Error(`unknown argument: ${argument}`)
  }
  return { check, outputRoot }
}
