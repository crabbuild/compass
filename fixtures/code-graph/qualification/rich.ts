function sealed<T extends { new (...args: any[]): object }>(constructor: T) {
  return constructor;
}

export interface Store {
  read(id: string): string;
}

@sealed
export class MemoryStore implements Store {
  read(id: string): string {
    return id;
  }
}

export { MemoryStore as DefaultStore };
