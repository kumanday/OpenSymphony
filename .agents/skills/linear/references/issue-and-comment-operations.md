# Issue And Comment Operations

Use these GraphQL operations when MCP is unavailable or when the required
mutation is not covered by MCP.

## Query an issue by key

```graphql
query IssueByKey($key: String!) {
  issue(id: $key) {
    id
    identifier
    title
    state {
      id
      name
      type
    }
    project {
      id
      name
      slugId
    }
    branchName
    url
    description
    updatedAt
  }
}
```

## Query team workflow states for an issue

Use this before `issueUpdate` when you need the exact destination `stateId`.

```graphql
query IssueTeamStates($id: String!) {
  issue(id: $id) {
    id
    team {
      id
      key
      name
      states {
        nodes {
          id
          name
          type
        }
      }
    }
  }
}
```

## Create a comment

```graphql
mutation CreateComment($issueId: String!, $body: String!) {
  commentCreate(input: { issueId: $issueId, body: $body }) {
    success
    comment {
      id
      url
    }
  }
}
```

## Update an existing comment

```graphql
mutation UpdateComment($id: String!, $body: String!) {
  commentUpdate(id: $id, input: { body: $body }) {
    success
    comment {
      id
      body
    }
  }
}
```

## Move an issue to another state

```graphql
mutation MoveIssueToState($id: String!, $stateId: String!) {
  issueUpdate(id: $id, input: { stateId: $stateId }) {
    success
    issue {
      id
      identifier
      state {
        id
        name
      }
    }
  }
}
```

## Attach a GitHub PR

Prefer this over a generic URL attachment when linking a PR.

Use this when you need GitHub-specific attachment semantics. OpenSymphony's
current MCP `linear_link_pr` tool uses `attachmentLinkURL` instead.

```graphql
mutation AttachGitHubPR($issueId: String!, $url: String!, $title: String) {
  attachmentLinkGitHubPR(
    issueId: $issueId
    url: $url
    title: $title
    linkKind: links
  ) {
    success
    attachment {
      id
      title
      url
    }
  }
}
```

## Attach a generic URL

This is the mutation currently used behind OpenSymphony's MCP `linear_link_pr`
tool.

```graphql
mutation AttachURL($issueId: String!, $url: String!, $title: String) {
  attachmentLinkURL(issueId: $issueId, url: $url, title: $title) {
    success
    attachment {
      id
      title
      url
    }
  }
}
```

## Create an issue relation

Use this for blocker/dependency metadata when plain MCP tools are insufficient.

```graphql
mutation CreateIssueRelation($input: IssueRelationCreateInput!) {
  issueRelationCreate(input: $input) {
    success
    issueRelation {
      id
      type
    }
  }
}
```

Example variables:

```json
{
  "input": {
    "issueId": "blocked-issue-uuid",
    "relatedIssueId": "blocking-issue-uuid",
    "type": "blocks"
  }
}
```
