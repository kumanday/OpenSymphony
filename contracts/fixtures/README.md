# Seed Fixtures

Representative fixtures for the EntreVista AI MVP happy path and escalation scenarios.

All fixtures are scoped by `tenant_id` and MUST NOT be used without an explicit `tenant_id` value.

## Fixture Index

| Fixture | File | Purpose |
|---------|------|---------|
| Tenant | `tenant_demo.json` | Multi-tenant organization context |
| Operator | `operator_recruiter.json` | Dashboard user for authentication |
| Campaign | `campaign_software_engineer.json` | Software engineering interview campaign |
| Rubric | `rubric_software_engineer.json` | Scoring rubric with competencies |
| Session | `session_happy_path.json` | Complete screening session flow |
| Session | `session_escalation.json` | Session with human escalation |
| Messages | `messages_happy_path.json` | Conversation transcript |
| Consent | `consent_record.json` | Candidate consent record |
| Evaluation | `evaluation_scored.json` | Completed evaluation with scores |
| Audit Event | `audit_events.json` | Representative audit log entries |
| Escalation | `escalation_alert.json` | Human escalation alert |
| NPS | `nps_submission.json` | Net Promoter Score submission |
| Document | `document_knowledge_base.json` | KB document for RAG |
