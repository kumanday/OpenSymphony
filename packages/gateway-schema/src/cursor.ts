/** Monotonic stream cursor for replay and resumable subscriptions. */
export interface StreamCursor {
  sequence: number;
  partition: string;
  timestamp_anchor?: number;
}

export function streamCursor(
  sequence: number,
  partition: string,
  timestamp_anchor?: number,
): StreamCursor {
  return { sequence, partition, timestamp_anchor };
}

/** Pagination cursor for detail reads. */
export interface PageCursor {
  /** Gateway-issued numeric sequence token encoded as a string. */
  page_token?: string;
  page_size: number;
}

/** Return a cursor that requests the first page (token omitted = start). */
export function pageCursorFirst(pageSize: number): PageCursor {
  return { page_size: pageSize };
}
