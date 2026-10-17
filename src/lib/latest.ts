/**
 * Request sequencing per key (a review, the list): reads of the same thing
 * can answer out of order, and an older answer landing last would put stale
 * data back (an old head's files). `begin` numbers a read; its `current()`
 * is false once a newer read of the same key started, and the caller then
 * drops the answer.
 */
export function latestOnly<K = string>() {
  const last = new Map<K, number>();
  return {
    begin(key: K): () => boolean {
      const n = (last.get(key) ?? 0) + 1;
      last.set(key, n);
      return () => last.get(key) === n;
    },
    /**
     * Note where every key stands now; the returned `moved(key)` is true
     * once a read of that key began after this call. A read of a whole
     * collection uses it to keep the members re-read meanwhile.
     */
    mark(): (key: K) => boolean {
      const at = new Map(last);
      return (key) => (last.get(key) ?? 0) !== (at.get(key) ?? 0);
    },
  };
}
