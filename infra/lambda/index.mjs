// Lambda handler — issues a pair of S3 presigned URLs so the Apple
// Shortcut can upload a file directly to S3 (PUT URL) and the Vercel
// browser app can fetch it once (GET URL). Lambda is only invoked
// for the small JSON signing request — actual file bytes never touch
// it, which sidesteps the 6 MB Function URL body limit.
//
// Flow:
//   1. POST {filename, contentType}  → returns {uploadUrl, fetchUrl, filename}
//   2. Apple Shortcut PUTs bytes to uploadUrl (5 GB max)
//   3. Shortcut opens https://<vercel>/?fetch=<encoded fetchUrl>
//   4. Browser fetches fetchUrl, reads Content-Disposition for filename
//   5. S3 lifecycle deletes the object after 1 hour (defense in depth
//      on top of the 30-min presigned GET expiry)

import {
  S3Client,
  PutObjectCommand,
  GetObjectCommand,
} from "@aws-sdk/client-s3";
import { getSignedUrl } from "@aws-sdk/s3-request-presigner";
import { randomUUID } from "node:crypto";

const BUCKET = process.env.BUCKET_NAME;
const REGION = process.env.AWS_REGION;
const PUT_TTL_SEC = 60 * 15; // 15 min — shortcut should upload almost immediately
const GET_TTL_SEC = 60 * 30; // 30 min — small window for browser to fetch

const s3 = new S3Client({ region: REGION });

// Tight allowlist of origins the signing endpoint will service. The
// preview/canary subdomains are included so PR-preview deployments
// of the Vercel project keep working.
const ALLOWED_ORIGIN_RE =
  /^https:\/\/(ts-hwp-transpiler[a-z0-9-]*\.vercel\.app|localhost(:\d+)?)$/;

function corsHeaders(origin) {
  const allow = ALLOWED_ORIGIN_RE.test(origin) ? origin : "null";
  return {
    "access-control-allow-origin": allow,
    "access-control-allow-methods": "POST, OPTIONS",
    "access-control-allow-headers": "content-type",
    "access-control-max-age": "86400",
    vary: "origin",
  };
}

function sanitizeFilename(name) {
  if (typeof name !== "string" || name.length === 0) return "shared.hwp";
  // Strip path traversal, quotes, and non-printable characters.
  return name
    .replace(/[\\/\x00-\x1f"]/g, "_")
    .slice(0, 200);
}

export const handler = async (event) => {
  const origin = event.headers?.origin || event.headers?.Origin || "";
  const method = event.requestContext?.http?.method || event.httpMethod;
  const cors = corsHeaders(origin);

  if (method === "OPTIONS") {
    return { statusCode: 204, headers: cors };
  }

  if (method !== "POST") {
    return {
      statusCode: 405,
      headers: { ...cors, "content-type": "text/plain" },
      body: "method not allowed",
    };
  }

  let payload = {};
  try {
    payload = event.body ? JSON.parse(event.body) : {};
  } catch {
    // empty body or non-JSON — fine, fall back to defaults
  }

  const filename = sanitizeFilename(payload.filename);
  const contentType =
    typeof payload.contentType === "string" && payload.contentType.length > 0
      ? payload.contentType
      : "application/octet-stream";

  const token = randomUUID();
  const key = `uploads/${token}`;

  // Apple Shortcut will PUT directly to S3 using this URL.
  const uploadUrl = await getSignedUrl(
    s3,
    new PutObjectCommand({
      Bucket: BUCKET,
      Key: key,
      ContentType: contentType,
    }),
    { expiresIn: PUT_TTL_SEC },
  );

  // Browser fetches via this URL. The S3 response will carry
  // Content-Disposition so the page can recover the filename.
  const fetchUrl = await getSignedUrl(
    s3,
    new GetObjectCommand({
      Bucket: BUCKET,
      Key: key,
      ResponseContentDisposition: `attachment; filename="${filename}"`,
    }),
    { expiresIn: GET_TTL_SEC },
  );

  return {
    statusCode: 200,
    headers: {
      ...cors,
      "content-type": "application/json",
      "cache-control": "no-store",
    },
    body: JSON.stringify({
      uploadUrl,
      fetchUrl,
      filename,
      expiresInSec: GET_TTL_SEC,
    }),
  };
};
