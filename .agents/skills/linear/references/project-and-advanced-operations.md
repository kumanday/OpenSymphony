# Project And Advanced Operations

OpenSymphony stores the Linear `Project.slugId` in `tracker.project_slug`.

Use these query files for project-side or advanced operations:

- Lookup a project by slug: `queries/project_by_slug.graphql`
- Update project overview/content: `queries/project_update_content.graphql`
- Set a project's current status: `queries/project_set_status.graphql`
- Create a reusable project status definition: `queries/project_status_create.graphql`
- Update a reusable project status definition: `queries/project_status_update.graphql`
- Request a signed upload target: `queries/file_upload.graphql`
- List available mutation names: `queries/introspect_mutations.graphql`
- Inspect an input object shape: `queries/introspect_input_shape.graphql`

## Notes

- `projectUpdate` accepts both `content` and `statusId`.
- `projectStatusCreate` requires `name`, `color`, `position`, and `type`.
- Project status names must be 25 characters or fewer.
- `fileUpload` only returns the signed upload target and partial header set.
  The actual `PUT` happens separately against `uploadUrl`, and the request must
  include `Content-Type` matching the `contentType` you requested in addition
  to the headers returned by Linear.
- For input-shape discovery, pass variables such as:

```json
{"name":"ProjectStatusUpdateInput"}
```
