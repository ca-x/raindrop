export function emitValidators(schemas, aliases = {}) {
  const validators = Object.entries(schemas).map(([name, schema]) => {
    const expression = emitCheck(schema, "value", name, 0)
    return [
      `export function is${name}(value: unknown): value is ${name} {`,
      `  return ${expression}`,
      "}",
    ].join("\n")
  })
  const aliasValidators = Object.entries(aliases).map(([alias, target]) =>
    [
      `export function is${alias}(value: unknown): value is ${alias} {`,
      `  return is${target}(value)`,
      "}",
    ].join("\n"),
  )

  return [helperFunctions(), ...validators, ...aliasValidators].join("\n\n")
}

function emitCheck(schema, value, path, depth) {
  assertSchema(schema, path)
  if (typeof schema.$ref === "string") {
    return `is${localRefName(schema.$ref, path)}(${value})`
  }
  if (Array.isArray(schema.enum)) {
    return parenthesize(
      schema.enum.map((item) => `${value} === ${JSON.stringify(item)}`).join(" || "),
    )
  }

  let primary = "true"
  if (schema.type !== undefined) {
    const types = Array.isArray(schema.type) ? schema.type : [schema.type]
    primary = parenthesize(
      types
        .map((type) => emitPrimitiveCheck(type, schema, value, path, depth))
        .join(" || "),
    )
  } else if (isObject(schema.properties) || Array.isArray(schema.required)) {
    primary = emitObjectCheck(schema, value, path, depth)
  }

  if (Array.isArray(schema.anyOf)) {
    const anyOf = parenthesize(
      schema.anyOf
        .map((candidate, index) =>
          emitCheck(candidate, value, `${path}.anyOf[${index}]`, depth + 1),
        )
        .join(" || "),
    )
    return primary === "true" ? anyOf : `${primary} && ${anyOf}`
  }
  return primary
}

function emitPrimitiveCheck(type, schema, value, path, depth) {
  switch (type) {
    case "null":
      return `${value} === null`
    case "string":
      return emitStringCheck(schema, value)
    case "integer":
      return emitNumberCheck(schema, value, true)
    case "number":
      return emitNumberCheck(schema, value, false)
    case "boolean":
      return `typeof ${value} === "boolean"`
    case "array": {
      assertSchema(schema.items, `${path}.items`)
      const item = `item${depth}`
      const checks = [`Array.isArray(${value})`]
      if (Number.isInteger(schema.minItems)) checks.push(`${value}.length >= ${schema.minItems}`)
      if (Number.isInteger(schema.maxItems)) checks.push(`${value}.length <= ${schema.maxItems}`)
      checks.push(
        `${value}.every((${item}) => ${emitCheck(schema.items, item, `${path}.items`, depth + 1)})`,
      )
      return parenthesize(checks.join(" && "))
    }
    case "object":
      return emitObjectCheck(schema, value, path, depth)
    default:
      throw new Error(`${path} uses unsupported type ${JSON.stringify(type)}`)
  }
}

function emitStringCheck(schema, value) {
  const checks = [`typeof ${value} === "string"`]
  if (Number.isInteger(schema.minLength)) checks.push(`${value}.length >= ${schema.minLength}`)
  if (Number.isInteger(schema.maxLength)) checks.push(`${value}.length <= ${schema.maxLength}`)
  if (typeof schema.pattern === "string") {
    checks.push(`new RegExp(${JSON.stringify(schema.pattern)}).test(${value})`)
  }
  if (schema.format === "uuid") checks.push(`isUuid(${value})`)
  if (schema.format === "uri") checks.push(`isUri(${value})`)
  return parenthesize(checks.join(" && "))
}

function emitNumberCheck(schema, value, integer) {
  const checks = [
    `typeof ${value} === "number"`,
    `Number.isFinite(${value})`,
  ]
  if (integer) checks.push(`Number.isInteger(${value})`)
  if (typeof schema.minimum === "number") checks.push(`${value} >= ${schema.minimum}`)
  if (typeof schema.maximum === "number") checks.push(`${value} <= ${schema.maximum}`)
  return parenthesize(checks.join(" && "))
}

function emitObjectCheck(schema, value, path, depth) {
  const checks = [`isRecord(${value})`]
  const properties = isObject(schema.properties) ? schema.properties : {}
  const required = new Set(Array.isArray(schema.required) ? schema.required : [])
  const allowed = Object.keys(properties)

  if (schema.additionalProperties === false) {
    checks.push(`hasOnlyKeys(${value}, ${JSON.stringify(allowed)})`)
  }
  if (Number.isInteger(schema.minProperties)) {
    checks.push(`Object.keys(${value}).length >= ${schema.minProperties}`)
  }
  for (const [field, fieldSchema] of Object.entries(properties)) {
    const fieldValue = `${value}[${JSON.stringify(field)}]`
    const validation = emitCheck(fieldSchema, fieldValue, `${path}.${field}`, depth + 1)
    checks.push(
      required.has(field)
        ? `hasOwn(${value}, ${JSON.stringify(field)}) && ${validation}`
        : `(!hasOwn(${value}, ${JSON.stringify(field)}) || ${validation})`,
    )
  }
  for (const field of required) {
    if (!Object.hasOwn(properties, field)) {
      checks.push(`hasOwn(${value}, ${JSON.stringify(field)})`)
    }
  }
  if (isObject(schema.additionalProperties)) {
    const entry = `entry${depth}`
    const item = `item${depth}`
    const extraCheck = emitCheck(
      schema.additionalProperties,
      item,
      `${path}.additionalProperties`,
      depth + 1,
    )
    const knownProperty =
      allowed.length === 0 ? "" : `${JSON.stringify(allowed)}.includes(${entry}) || `
    checks.push(
      `Object.entries(${value}).every(([${entry}, ${item}]) => ${knownProperty}${extraCheck})`,
    )
  }
  return parenthesize(checks.join(" && "))
}

function helperFunctions() {
  return [
    "function isRecord(value: unknown): value is Record<string, unknown> {",
    '  return typeof value === "object" && value !== null && !Array.isArray(value)',
    "}",
    "",
    "function hasOwn(value: Record<string, unknown>, key: string): boolean {",
    "  return Object.prototype.hasOwnProperty.call(value, key)",
    "}",
    "",
    "function hasOnlyKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {",
    "  return Object.keys(value).every((key) => keys.includes(key))",
    "}",
    "",
    "function isUuid(value: string): boolean {",
    "  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value)",
    "}",
    "",
    "function isUri(value: string): boolean {",
    "  try {",
    "    return new URL(value).protocol.length > 0",
    "  } catch {",
    "    return false",
    "  }",
    "}",
  ].join("\n")
}

function localRefName(reference, path) {
  const prefix = "#/components/schemas/"
  if (!reference.startsWith(prefix)) {
    throw new Error(`${path} uses unsupported reference ${reference}`)
  }
  return reference.slice(prefix.length)
}

function parenthesize(expression) {
  return `(${expression})`
}

function assertSchema(value, path) {
  if (!isObject(value)) throw new Error(`${path} must be a schema object`)
}

function isObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}
