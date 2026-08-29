import { writeFile } from "node:fs/promises"
import { spawn } from "node:child_process"

const separator = process.argv.indexOf("--")
const [outputPath, timeoutRaw, ...privatePaths] = process.argv.slice(2, separator)
const command = process.argv.slice(separator + 1)
const timeoutSeconds = Number.parseInt(timeoutRaw ?? "", 10)
if (!outputPath || separator < 0 || command.length === 0) {
  throw new Error("bounded_redacted_log requires OUTPUT TIMEOUT [PRIVATE_PATHS] -- COMMAND")
}
if (!Number.isSafeInteger(timeoutSeconds) || timeoutSeconds < 1 || timeoutSeconds > 3600) {
  throw new Error("bounded_redacted_log timeout must be 1-3600 seconds")
}

const maximumBytes = 64 * 1024
const captureLimit = maximumBytes + 4 * 1024
const chunks = []
let capturedBytes = 0
let truncated = false
let timedOut = false

const child = spawn(command[0], command.slice(1), {
  env: process.env,
  stdio: ["inherit", "pipe", "pipe"],
})
const capture = (chunk) => {
  const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk)
  const remaining = captureLimit - capturedBytes
  if (remaining > 0) {
    chunks.push(bytes.subarray(0, remaining))
    capturedBytes += Math.min(bytes.length, remaining)
  }
  if (bytes.length > remaining) truncated = true
}
child.stdout.on("data", capture)
child.stderr.on("data", capture)

const timer = setTimeout(() => {
  timedOut = true
  child.kill("SIGTERM")
  setTimeout(() => child.kill("SIGKILL"), 2_000).unref()
}, timeoutSeconds * 1_000)

const outcome = await new Promise((resolve, reject) => {
  child.once("error", reject)
  child.once("close", (code, signal) => resolve({ code, signal }))
})
clearTimeout(timer)

let rendered = Buffer.concat(chunks).toString("utf8")
for (const value of privatePaths.filter(Boolean).sort((left, right) => right.length - left.length)) {
  rendered = rendered.replaceAll(value, "<private-path>")
}
rendered = rendered
  .replaceAll(/\bsk-[A-Za-z0-9_-]+\b/g, "<redacted-secret>")
  .replaceAll(/\bBearer\s+[^\s"']*/gi, "Bearer <redacted-secret>")
  .replaceAll(
    /\b(api[_-]?key|token|password)(["']?\s*[:=]\s*["']?)[^\s"',}]*/gi,
    "$1$2<redacted-secret>",
  )
  .replaceAll(/\/(?:Users|home)\/[^/\s]+/g, "/<private-home>")
if (rendered.length > maximumBytes) {
  rendered = rendered.slice(0, maximumBytes)
  truncated = true
}
if (timedOut) rendered += `\n<stage timed out after ${timeoutSeconds} seconds>\n`
else if (truncated) rendered += "\n<log truncated at 65536 characters>\n"
await writeFile(outputPath, rendered, { encoding: "utf8", mode: 0o600 })

if (timedOut) process.exit(124)
if (typeof outcome.code === "number") process.exit(outcome.code)
process.exit(1)
