# Service Repository Template Notes

Standard conventions for all service repositories in the EntreVista AI system.

## Universal Conventions

### Environment Variable Naming

All environment variables follow this pattern:

```
<SERVICE_PREFIX>_<DESCRIPTIVE_NAME>
```

Rules:
- Uppercase with underscores only
- Sensitive values MUST come from AWS Secrets Manager, not plain `.env`
- `.env.example` MUST be committed to the repository with placeholder values
- `.env` MUST be in `.gitignore`
- All variables MUST be documented in `.env.example` with inline comments

### Correlation ID Propagation

Every service MUST:
1. Read `X-Correlation-ID` from incoming requests
2. If missing, generate a new UUIDv4
3. Propagate the ID to ALL outgoing requests
4. Include the ID in ALL log entries
5. Include the ID in ALL audit events written to compliance-lambda

### Standard Makefile Targets

Every service repository MUST include these targets:

```makefile
.PHONY: install lint format test test-contract build run clean

# Install dependencies
install:
    <package-manager install>

# Run linter (ruff for Python, eslint for Node)
lint:
    <linter command>

# Run formatter (ruff format for Python, prettier for Node)
format:
    <formatter command>

# Run unit tests
test:
    <test-runner command>

# Run contract tests against mock handlers (see contract-test-plan.md)
test-contract:
    <contract-test-runner command>

# Build production artifact
build:
    <build command>

# Run service locally
run:
    <run command>

# Clean build artifacts and caches
clean:
    rm -rf .pytest_cache __pycache__ build dist *.egg-info node_modules
    find . -type d -name '__pycache__' -exec rm -rf {} + 2>/dev/null || true
    find . -type f -name '*.pyc' -delete 2>/dev/null || true
```

### Error Response Envelope

All services return errors in this standard JSON structure:

```json
{
  "error": {
    "code": "VALIDATION_ERROR | UNAUTHORIZED | FORBIDDEN | NOT_FOUND | CONFLICT | RATE_LIMITED | INTERNAL_ERROR | SERVICE_UNAVAILABLE | TIMEOUT",
    "message": "Human-readable error description",
    "details": {}
  }
}
```

---

## Python/FastAPI Lambda Template (SVC-02, SVC-03, SVC-04, SVC-05, SVC-06)

### Repository Structure

```
service-name/
├── src/
│   ├── __init__.py
│   ├── handler.py          # Lambda entry point (Mangum wrapper)
│   ├── main.py             # FastAPI app definition
│   ├── config.py           # Settings with pydantic-settings
│   ├── dependencies.py     # Auth and correlation ID dependencies
│   ├── routers/
│   │   ├── __init__.py
│   │   └── <domain>.py     # Route handlers
│   ├── services/
│   │   ├── __init__.py
│   │   └── <domain>.py     # Business logic
│   ├── models/
│   │   ├── __init__.py
│   │   └── <domain>.py     # Pydantic models
│   └── middleware/
│       ├── __init__.py
│       ├── auth.py         # X-Internal-Secret / JWT validation
│       └── correlation.py  # X-Correlation-ID propagation
├── tests/
│   ├── __init__.py
│   ├── conftest.py         # Pytest fixtures
│   ├── test_contracts.py   # Contract validation tests
│   ├── test_routers/
│   └── test_services/
├── contracts/              # Symlink or copy of shared contracts
├── fixtures/               # Test data fixtures
├── .env.example
├── .gitignore
├── Makefile
├── pyproject.toml
├── Dockerfile              # Optional: for local testing
└── README.md
```

### pyproject.toml Essentials

```toml
[project]
name = "service-name"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = [
    "fastapi>=0.115.0",
    "mangum>=0.19.0",
    "pydantic>=2.0",
    "pydantic-settings>=2.0",
    "uvicorn>=0.34.0",
    "httpx>=0.28.0",
    "motor>=3.6.0",
    "pyjwt>=2.9.0",
]

[project.optional-dependencies]
dev = [
    "ruff>=0.9.0",
    "pytest>=8.0",
    "pytest-asyncio>=0.25",
    "pytest-mock>=3.14",
    "jsonschema>=4.23",
]

[tool.ruff]
line-length = 100
target-version = "py312"

[tool.ruff.lint]
select = ["E", "F", "I", "N", "W", "UP", "B", "SIM"]

[tool.pytest.ini_options]
asyncio_mode = "auto"
testpaths = ["tests"]
```

### .env.example

```bash
# MongoDB
MONGODB_URI=mongodb+srv://<user>:<password>@<cluster>.mongodb.net/<database>?retryWrites=true&w=majority

# Anthropic (for services that use Claude)
ANTHROPIC_API_KEY=sk-ant-<placeholder>
CLAUDE_MODEL_ID=claude-opus-4-6

# Internal service auth
INTERNAL_SERVICE_SECRET=<shared-secret-from-secrets-manager>

# Service URLs (for inter-service calls)
CONVERSATION_LAMBDA_URL=https://<api-id>.execute-api.<region>.amazonaws.com/prod
EVALUATION_LAMBDA_URL=https://<api-id>.execute-api.<region>.amazonaws.com/prod
CAMPAIGN_LAMBDA_URL=https://<api-id>.execute-api.<region>.amazonaws.com/prod
COMPLIANCE_LAMBDA_URL=https://<api-id>.execute-api.<region>.amazonaws.com/prod
AUTH_LAMBDA_URL=https://<api-id>.execute-api.<region>.amazonaws.com/prod

# JWT (for services that validate dashboard tokens)
JWT_PUBLIC_KEY=<path-to-public-key-or-placeholder>
```

### handler.py Pattern

```python
from mangum import Mangum
from src.main import app

lambda_handler = Mangum(app, lifespan="auto")
```

### Auth Middleware Pattern

```python
import os
from fastapi import Request, HTTPException, status
from fastapi.security import HTTPBearer, HTTPAuthorizationCredentials

internal_secret = os.getenv("INTERNAL_SERVICE_SECRET", "")

async def verify_internal_auth(request: Request):
    """Validate X-Internal-Secret header for service-to-service calls."""
    secret = request.headers.get("X-Internal-Secret")
    if not secret or secret != internal_secret:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail={"error": {"code": "UNAUTHORIZED", "message": "Invalid X-Internal-Secret"}},
        )

async def verify_jwt_auth(
    credentials: HTTPAuthorizationCredentials = HTTPBearer(),
):
    """Validate JWT Bearer token for dashboard-initiated calls."""
    # Implementation: decode JWT, validate signature with public key,
    # check exp, extract operator_id and tenant_id claims
    pass
```

### Correlation ID Middleware Pattern

```python
import uuid
from fastapi import Request
from starlette.middleware.base import BaseHTTPMiddleware

class CorrelationIdMiddleware(BaseHTTPMiddleware):
    async def dispatch(self, request: Request, call_next):
        correlation_id = request.headers.get(
            "X-Correlation-ID", str(uuid.uuid4())
        )
        request.state.correlation_id = correlation_id
        response = await call_next(request)
        response.headers["X-Correlation-ID"] = correlation_id
        return response
```

---

## Node.js/Telegraf Template (SVC-01)

### Repository Structure

```
telegram-bot/
├── src/
│   ├── handler.ts          # Lambda entry point
│   ├── bot.ts              # Telegraf bot instance
│   ├── webhook.ts          # Webhook handler
│   ├── config.ts           # Configuration
│   ├── middleware/
│   │   ├── auth.ts         # Internal secret validation
│   │   └── correlation.ts  # Correlation ID propagation
│   ├── services/
│   │   └── conversation.ts # Service client for conversation-lambda
│   └── types/
│       └── index.ts        # TypeScript type definitions
├── tests/
│   ├── handler.test.ts
│   └── services/
│       └── conversation.test.ts
├── contracts/              # Symlink or copy of shared contracts
├── fixtures/               # Test data fixtures
├── .env.example
├── .gitignore
├── Makefile
├── package.json
├── tsconfig.json
└── README.md
```

### package.json Essentials

```json
{
  "name": "telegram-bot",
  "version": "0.1.0",
  "main": "dist/handler.js",
  "scripts": {
    "build": "tsc",
    "start": "node dist/handler.js",
    "dev": "ts-node-dev --respawn src/handler.ts",
    "lint": "eslint src/",
    "format": "prettier --write src/",
    "test": "jest",
    "test:contract": "jest tests/contracts/",
    "clean": "rm -rf dist node_modules"
  },
  "dependencies": {
    "telegraf": "^4.16.0",
    "axios": "^1.7.0",
    "uuid": "^11.0.0"
  },
  "devDependencies": {
    "@types/node": "^22.0.0",
    "typescript": "^5.7.0",
    "eslint": "^9.0.0",
    "prettier": "^3.4.0",
    "jest": "^29.7.0",
    "ts-jest": "^29.2.0",
    "ts-node-dev": "^2.0.0"
  }
}
```

### .env.example

```bash
# Telegram
TELEGRAM_BOT_TOKEN=<bot-token-from-botfather>
TELEGRAM_WEBHOOK_SECRET=<webhook-secret-token>

# Internal service calls
CONVERSATION_LAMBDA_URL=https://<api-id>.execute-api.<region>.amazonaws.com/prod
INTERNAL_SERVICE_SECRET=<shared-secret-from-secrets-manager>

# AWS (for local development only; in Lambda these come from environment)
AWS_REGION=us-east-1
AWS_ACCESS_KEY_ID=<placeholder>
AWS_SECRET_ACCESS_KEY=<placeholder>
```

### Webhook Handler Pattern

```typescript
import { Bot, webhookCallback } from 'grammy';
import { APIGatewayProxyEvent, APIGatewayProxyResult } from 'aws-lambda';

const bot = new Bot(process.env.TELEGRAM_BOT_TOKEN!);

export const handler = async (
  event: APIGatewayProxyEvent,
): Promise<APIGatewayProxyResult> => {
  // Verify webhook secret
  const secretToken = event.headers['x-telegram-bot-api-secret-token'];
  if (secretToken !== process.env.TELEGRAM_WEBHOOK_SECRET) {
    return { statusCode: 401, body: 'Unauthorized' };
  }

  // Parse update
  const update = JSON.parse(event.body || '{}');

  // Always return 200 to prevent Telegram retry storms
  try {
    await bot.handleUpdate(update);
    return { statusCode: 200, body: 'OK' };
  } catch (error) {
    console.error('Webhook error:', error);
    return { statusCode: 200, body: 'OK' }; // Still 200 to prevent retries
  }
};
```

---

## React/Vite Template (SVC-07)

### Repository Structure

```
dashboard/
├── src/
│   ├── main.tsx
│   ├── App.tsx
│   ├── vite-env.d.ts
│   ├── api/
│   │   ├── client.ts          # HTTP client with JWT injection
│   │   ├── auth.ts            # Auth API calls
│   │   ├── campaigns.ts       # Campaign API calls
│   │   ├── evaluations.ts     # Evaluation API calls
│   │   └── compliance.ts      # Compliance API calls
│   ├── components/
│   ├── pages/
│   │   ├── Login.tsx
│   │   ├── ReviewQueue.tsx
│   │   ├── CandidateDetail.tsx
│   │   ├── CampaignManager.tsx
│   │   ├── KnowledgeBase.tsx
│   │   └── Analytics.tsx
│   ├── hooks/
│   │   ├── useAuth.ts
│   │   └── useApi.ts
│   ├── stores/
│   │   └── authStore.ts       # In-memory JWT storage
│   ├── types/
│   │   └── index.ts
│   └── utils/
│       └── correlation.ts     # Correlation ID generation
├── tests/
│   ├── contract-tests/
│   └── components/
├── contracts/                  # Symlink or copy of shared contracts
├── .env.example
├── .env.production
├── .gitignore
├── Makefile
├── index.html
├── package.json
├── tsconfig.json
├── vite.config.ts
└── README.md
```

### package.json Essentials

```json
{
  "name": "entrevisata-dashboard",
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview",
    "lint": "eslint src/",
    "format": "prettier --write src/",
    "test": "vitest",
    "test:contract": "vitest tests/contract-tests/",
    "clean": "rm -rf dist node_modules"
  },
  "dependencies": {
    "react": "^18.3.0",
    "react-dom": "^18.3.0",
    "react-router-dom": "^7.1.0",
    "axios": "^1.7.0",
    "zustand": "^5.0.0",
    "uuid": "^11.0.0"
  },
  "devDependencies": {
    "@types/react": "^18.3.0",
    "@types/react-dom": "^18.3.0",
    "@vitejs/plugin-react": "^4.3.0",
    "typescript": "^5.7.0",
    "vite": "^6.0.0",
    "vitest": "^3.0.0",
    "eslint": "^9.0.0",
    "prettier": "^3.4.0"
  }
}
```

### .env.example

```bash
# API Gateway base URL for all lambda calls
VITE_API_BASE_URL=https://<api-id>.execute-api.<region>.amazonaws.com/prod

# Build-time configuration
VITE_APP_NAME=EntreVista AI Dashboard
VITE_APP_VERSION=0.1.0
```

### Security Patterns

```typescript
// JWT stored in memory ONLY (never localStorage)
// Refresh token in HTTP-only cookie (set by auth-lambda)
// All API calls use HTTPS (enforced by API Gateway)

// API Client with JWT injection
import axios from 'axios';
import { useAuthStore } from '../stores/authStore';

const apiClient = axios.create({
  baseURL: import.meta.env.VITE_API_BASE_URL,
  timeout: 30000,
});

apiClient.interceptors.request.use((config) => {
  const { accessToken } = useAuthStore.getState();
  if (accessToken) {
    config.headers.Authorization = `Bearer ${accessToken}`;
  }
  // Add correlation ID
  config.headers['X-Correlation-ID'] = crypto.randomUUID();
  return config;
});

// 401 handling with automatic token refresh
apiClient.interceptors.response.use(
  (response) => response,
  async (error) => {
    if (error.response?.status === 401) {
      // Trigger token refresh
      // If refresh fails, redirect to login
    }
    // Surface user-visible toast (no raw error messages)
    return Promise.reject(error);
  }
);
```

---

## Contract References

All downstream service tasks should reference these contract artifacts before coding:

| Service | Contracts to Reference |
|---------|----------------------|
| telegram-bot (SVC-01) | C-01 |
| conversation-lambda (SVC-02) | C-01, C-02, C-03, C-04, C-05, C-06 |
| evaluation-lambda (SVC-03) | C-02, C-05, C-09 |
| campaign-lambda (SVC-04) | C-03, C-05, C-08 |
| compliance-lambda (SVC-05) | C-04, C-05, C-06, C-10 |
| auth-lambda (SVC-06) | C-07 |
| dashboard (SVC-07) | C-07, C-08, C-09, C-10 |
