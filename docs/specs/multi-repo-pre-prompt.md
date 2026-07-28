Someone has been working on implementing a multi-repo evolution of OpenSymphony. One of the things that needed to be adapted was that the WORKFLOW.md file was inside of the target repo, and we needed a way to have the orchestrator read a file for scope, context, and execution across several repos, as well as having each agent working on a task to read the workflow instructions for its specific target repo.



There, I realized I needed to decouple several things that I had in the WORKFLOW.md that should actually be moved to the config.yaml because they were relevant to the orchestrator and not to the task implementation agents. I also needed to decouple what information, if any, was needed by the orchestrator in the WORKFLOW.md file that was ALSO needed for the implementation task agents, and what information was of separate concerns. We need to more clearly separate the context that is relevant to each part individually and to both of them. The other thing is that each implementation agent has a workspace that is a git checkout of its task's target repo.



So what I had in mind was to manage things through the existing hierarchical task system where terminal child tasks were bound to a specific repo and had a single repo's work defined in its scope. On the other hand, parent tasks could have scope across several repos, for example: a frontend repo and a backend repo. And so that parent task could have two child subtasks, one for each corresponding repo, and each subtask would have its own workspace for the relevant repo. The parent task would be charged with integration, verification, and any additional work, with the challenge that the parent task wouldn't really have a workspace of its own since its concerns spanned several repos. Or, on the other hand, it could and probably should use the workspaces of its child tasks. That way it could do implementation of any integration and verifications using the existing checkouts instead of having the overhead of recreating each workspace. So with the frontend running in the frontend workspace and the backend running in the backend workspace, the parent task agent could then perform integration tests and bug fixes. The cleanup lifecycle would have to be changed so that child tasks do not automatically clean up their workspaces, and it's up to the parent task to do final cleanup after any integration tests and additional work is done. Since the feature branches in the child repos would already have been merged when the parent task uses them (since each parent task is blocked until all its children are completed and merged), each workspace would need to have the target branch (e.g. develop) merged back into it, and a new fix branch created in the case of any needed additional changes, which would then go through a new PR review process as configured.



The work for this has been extensive and included some changes in taxonomy, including things like project sets, so that the orchestrator knows what linear projects to have in memory, and what repos are associated with each project. The developer that I delegated this work to has handed over an in-progress pull request. We need to do extensive code review of all the changes there and also figure out what missing functionality we still need to implement. I believe the parent task handling of child workspaces is something that is still pending.



This is an extensive research and analysis task that we need to first do a pass of reconnaissance to formulate a plan before executing it. Also, I want to leverage model diversity.

