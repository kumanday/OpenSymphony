import type { RunTimeline, TimelineEntry, TimelineEntryKind } from "@opensymphony/gateway-schema";

/**
 * Render a run timeline as a plain DOM tree.
 *
 * Returns a root element containing grouped timeline entries with stable
 * ids, titles, summaries, and entity links. Callers mount the element into
 * the document where they want the timeline visible.
 */
export function renderTimeline(timeline: RunTimeline): HTMLElement {
  const root = document.createElement("div");
  root.className = "run-timeline";
  root.setAttribute("data-run-id", timeline.run_id);
  root.setAttribute("data-schema-version", JSON.stringify(timeline.schema_version));

  if (timeline.entries.length === 0) {
    const empty = document.createElement("p");
    empty.className = "timeline-empty";
    empty.textContent = "No timeline events for this run.";
    root.appendChild(empty);
    return root;
  }

  const list = document.createElement("ol");
  list.className = "timeline-entries";
  for (const entry of timeline.entries) {
    list.appendChild(renderTimelineEntry(entry));
  }
  root.appendChild(list);
  return root;
}

function renderTimelineEntry(entry: TimelineEntry): HTMLElement {
  const item = document.createElement("li");
  item.className = `timeline-entry timeline-entry-${entry.kind}`;
  item.setAttribute("data-entry-id", entry.entry_id);
  item.setAttribute("data-sequence-start", entry.sequence_start.toString());
  item.setAttribute("data-sequence-end", entry.sequence_end.toString());

  const header = document.createElement("div");
  header.className = "timeline-entry-header";

  const kindBadge = document.createElement("span");
  kindBadge.className = "timeline-entry-kind";
  kindBadge.textContent = entry.kind;
  header.appendChild(kindBadge);

  if (entry.phase) {
    const phase = document.createElement("span");
    phase.className = `timeline-entry-phase timeline-entry-phase-${entry.phase}`;
    phase.textContent = entry.phase;
    header.appendChild(phase);
  }

  const title = document.createElement("strong");
  title.className = "timeline-entry-title";
  title.textContent = entry.title;
  header.appendChild(title);

  const time = document.createElement("time");
  time.className = "timeline-entry-time";
  time.setAttribute("datetime", entry.happened_at);
  time.textContent = entry.happened_at;
  header.appendChild(time);

  item.appendChild(header);

  const summary = document.createElement("p");
  summary.className = "timeline-entry-summary";
  summary.textContent = entry.summary;
  item.appendChild(summary);

  if (entry.entity_refs.length > 0) {
    const refs = document.createElement("ul");
    refs.className = "timeline-entry-refs";
    for (const ref of entry.entity_refs) {
      const li = document.createElement("li");
      li.textContent = ref.identifier
        ? `${ref.kind}: ${ref.identifier} (${ref.id})`
        : `${ref.kind}: ${ref.id}`;
      refs.appendChild(li);
    }
    item.appendChild(refs);
  }

  if (entry.state_evidence) {
    const evidence = document.createElement("details");
    evidence.className = "timeline-entry-evidence";
    const summaryEl = document.createElement("summary");
    summaryEl.textContent = `Evidence: ${entry.state_evidence.phase} / ${entry.state_evidence.stream}`;
    evidence.appendChild(summaryEl);

    const explanation = document.createElement("p");
    explanation.textContent = entry.state_evidence.explanation;
    evidence.appendChild(explanation);

    if (entry.state_evidence.last_activity_at) {
      const last = document.createElement("p");
      last.textContent = `Last activity: ${entry.state_evidence.last_activity_at}`;
      evidence.appendChild(last);
    }
    if (entry.state_evidence.stall_deadline_at) {
      const deadline = document.createElement("p");
      deadline.textContent = `Stall deadline: ${entry.state_evidence.stall_deadline_at}`;
      evidence.appendChild(deadline);
    }
    item.appendChild(evidence);
  }

  return item;
}

/**
 * Filter a timeline to entries matching one or more kinds.
 */
export function filterTimelineEntries(
  timeline: RunTimeline,
  kinds: TimelineEntryKind[],
): TimelineEntry[] {
  return timeline.entries.filter((e) => kinds.includes(e.kind));
}

/**
 * Find the first timeline entry for a specific event id.
 */
export function findTimelineEntryByEventId(
  timeline: RunTimeline,
  eventId: string,
): TimelineEntry | undefined {
  return timeline.entries.find((e) => e.event_ids.includes(eventId));
}

/**
 * Find the first timeline entry that references a given entity id.
 */
export function findTimelineEntryByEntityId(
  timeline: RunTimeline,
  id: string,
): TimelineEntry | undefined {
  return timeline.entries.find((e) => e.entity_refs.some((ref) => ref.id === id));
}
