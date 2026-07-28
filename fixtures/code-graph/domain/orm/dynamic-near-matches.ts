function Entity(value: string) {
  return (target: unknown) => target;
}

@Entity(getDynamicTable())
class NotTypeOrm {}
