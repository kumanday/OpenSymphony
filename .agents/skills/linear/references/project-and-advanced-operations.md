# Project And Advanced Operations

Use these references for GraphQL-only work that falls outside the current MCP
surface.

These are GraphQL-only today because the current OpenSymphony MCP surface does
not include project reads, project writes, uploads, or schema helpers.

## Find a project by slugId

OpenSymphony stores the Linear `Project.slugId` value in `tracker.project_slug`.

```graphql
query ProjectBySlug($slug: String!) {
  projects(filter: { slugId: { eq: $slug } }, first: 1) {
    nodes {
      id
      name
      slugId
      url
      description
      content
    }
  }
}
```

## Update project overview/content

Linear project overview markdown lives in `Project.content`.

```graphql
mutation UpdateProjectContent($id: String!, $content: String!) {
  projectUpdate(id: $id, input: { content: $content }) {
    success
    project {
      id
      name
      slugId
      content
      updatedAt
    }
  }
}
```

The live Linear schema also exposes `projectStatusCreate` and
`projectStatusUpdate`, but OpenSymphony does not yet ship dedicated MCP tools
or canned mutation examples for those operations. Introspect their input shapes
before first use.

## Upload a file for use in a comment

Do this in three steps:

1. Request a signed upload target with `fileUpload`.
2. Upload the local bytes to `uploadUrl` using the exact returned headers.
3. Create or update the comment with the returned `assetUrl`.

```graphql
mutation FileUpload(
  $filename: String!
  $contentType: String!
  $size: Int!
  $makePublic: Boolean
) {
  fileUpload(
    filename: $filename
    contentType: $contentType
    size: $size
    makePublic: $makePublic
  ) {
    success
    uploadFile {
      uploadUrl
      assetUrl
      headers {
        key
        value
      }
    }
  }
}
```

When shell upload is needed, only use it against the signed `uploadUrl`. Do not
invent a separate authenticated upload flow.

Linear's upload guide requires the actual `PUT` to happen server-side and to
copy the exact headers returned by `fileUpload`.

## Introspection helpers

Use these when the exact mutation or input type is unclear.

### List mutation names

```graphql
query ListMutations {
  __type(name: "Mutation") {
    fields {
      name
    }
  }
}
```

### Inspect an input object

```graphql
query InputShape($name: String!) {
  __type(name: $name) {
    inputFields {
      name
      type {
        kind
        name
        ofType {
          kind
          name
          ofType {
            kind
            name
          }
        }
      }
    }
  }
}
```

Useful input names:

- `ProjectUpdateInput`
- `CommentCreateInput`
- `CommentUpdateInput`
- `IssueUpdateInput`
- `IssueRelationCreateInput`

Use introspection before guessing unfamiliar project-update or document-related
mutation shapes.
