type Handler<T> = (value: T) => Promise<T>;

function transform(value: string): string;
function transform(value: number): number;
function transform(value: string | number): string | number {
  return value;
}

function traced<T extends new (...args: any[]) => object>(target: T): T {
  return target;
}

@traced
class Service {
  async execute<T>(value: T, callback: Handler<T>): Promise<T> {
    return callback(value);
  }
}

export async function run(
  service: Service,
  dynamicKey: string,
): Promise<JSX.Element> {
  const table: Record<string, () => number> = {
    local: () => transform(1),
  };
  const selected = table[dynamicKey];
  const result = selected?.() ?? 0;
  await service.execute(result, async (value) => value + 1);
  return <output data-result={result}>{transform("done")}</output>;
}
