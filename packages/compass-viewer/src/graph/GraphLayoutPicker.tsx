import {
  CheckIcon,
  ChevronDownIcon,
  ChevronUpIcon,
  CircleIcon,
  Grid2X2Icon,
  NetworkIcon,
  ShellIcon,
  TargetIcon
} from "lucide-react";
import { Select } from "radix-ui";
import type { GraphLayoutStyle } from "./renderingProfile";

const layouts = [
  { value: "automatic", label: "Automatic", Icon: NetworkIcon },
  { value: "circle", label: "Circle", Icon: CircleIcon },
  { value: "concentric", label: "Concentric", Icon: TargetIcon },
  { value: "spiral", label: "Spiral", Icon: ShellIcon },
  { value: "grid", label: "Square grid", Icon: Grid2X2Icon }
] as const;

export function GraphLayoutPicker({
  value,
  onChange,
  onOpen
}: {
  value: GraphLayoutStyle;
  onChange(value: GraphLayoutStyle): void;
  onOpen(): void;
}) {
  const selected = layouts.find((layout) => layout.value === value) ?? layouts[0];
  return (
    <Select.Root
      value={value}
      onValueChange={(next) => {
        const layout = layouts.find((option) => option.value === next);
        if (layout) onChange(layout.value);
      }}
      onOpenChange={(open) => { if (open) onOpen(); }}
    >
      <Select.Trigger className="compass-layout-picker" aria-label="Graph layout">
        <selected.Icon aria-hidden="true" />
        <Select.Value>{selected.label}</Select.Value>
        <Select.Icon asChild><ChevronDownIcon aria-hidden="true" /></Select.Icon>
      </Select.Trigger>
      <Select.Portal>
        <Select.Content
          className="compass-layout-menu"
          position="popper"
          align="start"
          sideOffset={6}
          collisionPadding={12}
          aria-label="Graph layout"
        >
          <Select.ScrollUpButton className="compass-layout-menu-scroll">
            <ChevronUpIcon aria-hidden="true" />
          </Select.ScrollUpButton>
          <Select.Viewport>
            {layouts.map(({ value: option, label, Icon }) => (
              <Select.Item key={option} value={option} className="compass-layout-option">
                <Icon aria-hidden="true" />
                <Select.ItemText>{label}</Select.ItemText>
                <Select.ItemIndicator className="compass-layout-check">
                  <CheckIcon aria-hidden="true" />
                </Select.ItemIndicator>
              </Select.Item>
            ))}
          </Select.Viewport>
          <Select.ScrollDownButton className="compass-layout-menu-scroll">
            <ChevronDownIcon aria-hidden="true" />
          </Select.ScrollDownButton>
        </Select.Content>
      </Select.Portal>
    </Select.Root>
  );
}
