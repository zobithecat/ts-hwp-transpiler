terraform {
  required_version = ">= 1.5.0"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.70"
    }
    archive = {
      source  = "hashicorp/archive"
      version = "~> 2.4"
    }
    random = {
      source  = "hashicorp/random"
      version = "~> 3.6"
    }
  }
}

provider "aws" {
  region = var.region
}

variable "region" {
  description = "AWS region for the share-relay stack"
  type        = string
  default     = "ap-northeast-2"
}

variable "project" {
  description = "Resource name prefix"
  type        = string
  default     = "hwp-share"
}

# Random suffix so the bucket name is globally unique without the
# user having to think one up.
resource "random_id" "bucket" {
  byte_length = 4
}

# -------------------------------------------------------------------
# S3 bucket — holds shared HWP files for at most 1 hour
# -------------------------------------------------------------------
resource "aws_s3_bucket" "share" {
  bucket        = "${var.project}-${random_id.bucket.hex}"
  force_destroy = true
}

resource "aws_s3_bucket_public_access_block" "share" {
  bucket                  = aws_s3_bucket.share.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_lifecycle_configuration" "share" {
  bucket = aws_s3_bucket.share.id

  rule {
    id     = "expire-uploads-after-1h"
    status = "Enabled"

    filter {
      prefix = "uploads/"
    }

    # S3 lifecycle is day-granular for expiration; 1 day is the
    # smallest knob. The 30-min presigned GET expiry is the real
    # primary defense — this rule is just to guarantee eventual
    # deletion even if a fetch never happens.
    expiration {
      days = 1
    }

    abort_incomplete_multipart_upload {
      days_after_initiation = 1
    }
  }
}

resource "aws_s3_bucket_cors_configuration" "share" {
  bucket = aws_s3_bucket.share.id

  cors_rule {
    allowed_methods = ["GET", "PUT", "HEAD"]
    allowed_origins = [
      "https://ts-hwp-transpiler.vercel.app",
      "https://*.vercel.app",
      "http://localhost:5173",
    ]
    allowed_headers = ["*"]
    expose_headers  = ["Content-Disposition", "Content-Length", "Content-Type"]
    max_age_seconds = 86400
  }
}

# -------------------------------------------------------------------
# Lambda — issues presigned URL pairs (presign-only, no file bytes)
# -------------------------------------------------------------------
data "aws_iam_policy_document" "lambda_assume" {
  statement {
    actions = ["sts:AssumeRole"]
    principals {
      type        = "Service"
      identifiers = ["lambda.amazonaws.com"]
    }
  }
}

resource "aws_iam_role" "lambda" {
  name               = "${var.project}-sign-role"
  assume_role_policy = data.aws_iam_policy_document.lambda_assume.json
}

resource "aws_iam_role_policy_attachment" "logs" {
  role       = aws_iam_role.lambda.name
  policy_arn = "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole"
}

data "aws_iam_policy_document" "s3_access" {
  statement {
    actions = [
      "s3:PutObject",
      "s3:GetObject",
    ]
    resources = ["${aws_s3_bucket.share.arn}/uploads/*"]
  }
}

resource "aws_iam_role_policy" "s3_access" {
  name   = "${var.project}-s3-access"
  role   = aws_iam_role.lambda.id
  policy = data.aws_iam_policy_document.s3_access.json
}

# Run `npm install --omit=dev` before zipping so the deps land in
# node_modules. The hash of package.json is used as a trigger so we
# only reinstall when deps change.
resource "null_resource" "npm_install" {
  triggers = {
    package_json = filemd5("${path.module}/lambda/package.json")
  }

  provisioner "local-exec" {
    working_dir = "${path.module}/lambda"
    command     = "npm install --omit=dev --no-audit --no-fund"
  }
}

data "archive_file" "lambda_zip" {
  type        = "zip"
  source_dir  = "${path.module}/lambda"
  output_path = "${path.module}/.terraform/lambda.zip"

  depends_on = [null_resource.npm_install]
}

resource "aws_lambda_function" "sign" {
  function_name    = "${var.project}-sign"
  role             = aws_iam_role.lambda.arn
  handler          = "index.handler"
  runtime          = "nodejs22.x"
  filename         = data.archive_file.lambda_zip.output_path
  source_code_hash = data.archive_file.lambda_zip.output_base64sha256
  timeout          = 10
  memory_size      = 256

  # Cap concurrency so a stray script can't run up the bill.
  reserved_concurrent_executions = 10

  environment {
    variables = {
      BUCKET_NAME = aws_s3_bucket.share.bucket
    }
  }
}

# -------------------------------------------------------------------
# API Gateway HTTP API — public POST /sign in front of the Lambda.
# Lambda Function URLs would be simpler but this account blocks
# public invocation on them, so we go through HTTP API instead.
# -------------------------------------------------------------------
resource "aws_apigatewayv2_api" "sign" {
  name          = "${var.project}-sign-api"
  protocol_type = "HTTP"

  cors_configuration {
    allow_origins = [
      "https://ts-hwp-transpiler.vercel.app",
      "http://localhost:5173",
    ]
    allow_methods = ["POST", "OPTIONS"]
    allow_headers = ["content-type"]
    max_age       = 86400
  }
}

resource "aws_apigatewayv2_integration" "sign" {
  api_id                 = aws_apigatewayv2_api.sign.id
  integration_type       = "AWS_PROXY"
  integration_uri        = aws_lambda_function.sign.invoke_arn
  integration_method     = "POST"
  payload_format_version = "2.0"
}

resource "aws_apigatewayv2_route" "sign" {
  api_id    = aws_apigatewayv2_api.sign.id
  route_key = "POST /sign"
  target    = "integrations/${aws_apigatewayv2_integration.sign.id}"
}

resource "aws_apigatewayv2_stage" "default" {
  api_id      = aws_apigatewayv2_api.sign.id
  name        = "$default"
  auto_deploy = true
}

resource "aws_lambda_permission" "apigw_invoke" {
  statement_id  = "AllowAPIGatewayInvoke"
  action        = "lambda:InvokeFunction"
  function_name = aws_lambda_function.sign.function_name
  principal     = "apigateway.amazonaws.com"
  source_arn    = "${aws_apigatewayv2_api.sign.execution_arn}/*/*"
}

# -------------------------------------------------------------------
# Outputs
# -------------------------------------------------------------------
output "sign_endpoint" {
  description = "POST {filename, contentType} here from Apple Shortcut"
  value       = "${aws_apigatewayv2_api.sign.api_endpoint}/sign"
}

output "bucket_name" {
  value = aws_s3_bucket.share.bucket
}
