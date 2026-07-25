export class LruCache<K, V> {
  private readonly values = new Map<K, V>();
  constructor(private readonly capacity: number) {
    if (capacity < 1) throw new Error("LRU capacity must be positive");
  }
  get(key: K): V | undefined {
    const value = this.values.get(key);
    if (value === undefined) return undefined;
    this.values.delete(key);
    this.values.set(key, value);
    return value;
  }
  set(key: K, value: V): void {
    this.values.delete(key);
    this.values.set(key, value);
    while (this.values.size > this.capacity) {
      const oldest = this.values.keys().next().value as K | undefined;
      if (oldest === undefined) break;
      this.values.delete(oldest);
    }
  }
  delete(key: K): void {
    this.values.delete(key);
  }
  keys(): K[] {
    return [...this.values.keys()].reverse();
  }
}
