import { mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { dirname, relative, resolve } from "node:path"

export function writeOutputs(outputRoot, outputs) {
  for (const output of outputs) {
    const path = resolve(outputRoot, output.path)
    mkdirSync(dirname(path), { recursive: true })
    writeFileSync(path, output.content)
    process.stdout.write(`generated ${relative(outputRoot, path)}\n`)
  }
}

export function checkOutputs(outputRoot, outputs) {
  const failures = []
  for (const output of outputs) {
    const path = resolve(outputRoot, output.path)
    let actual
    try {
      actual = readFileSync(path, "utf8")
    } catch (error) {
      if (error?.code === "ENOENT") {
        failures.push(`missing generated file: ${relative(outputRoot, path)}`)
        continue
      }
      throw error
    }
    if (actual !== output.content) {
      failures.push(`out-of-date generated file: ${relative(outputRoot, path)}`)
    }
  }
  if (failures.length === 0) {
    process.stdout.write("generated Reader contracts are current\n")
    return true
  }
  process.stderr.write(`${failures.join("\n")}\n`)
  return false
}
