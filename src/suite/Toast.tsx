import { useEffect, useState } from "react";

interface Props {
  message: string;
  type?: "success" | "error";
  duration?: number;
  onDone: () => void;
}

export function Toast({ message, type, duration = 3500, onDone }: Props) {
  const [visible, setVisible] = useState(true);

  useEffect(() => {
    let fade: ReturnType<typeof setTimeout> | undefined;
    const timer = setTimeout(() => {
      setVisible(false);
      fade = setTimeout(onDone, 300);
    }, duration);
    return () => {
      clearTimeout(timer);
      clearTimeout(fade);
    };
  }, [duration, onDone]);

  return (
    <div
      role="status"
      className={`toast ${visible ? "toast-in" : "toast-out"}${type ? ` toast-${type}` : ""}`}
    >
      {message}
    </div>
  );
}
