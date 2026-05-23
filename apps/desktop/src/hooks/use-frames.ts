import { useQuery } from "@tanstack/react-query";

import { frameDetail, framesList } from "@/lib/tauri";
import type { ListFramesArgs } from "@/lib/types";

export const FRAMES_QUERY_KEY = ["frames"] as const;

export function useFrames(args: ListFramesArgs = {}) {
  return useQuery({
    queryKey: [...FRAMES_QUERY_KEY, "list", args],
    queryFn: () => framesList(args),
    refetchInterval: 5000,
  });
}

export function useFrameDetail(id: string | null) {
  return useQuery({
    queryKey: [...FRAMES_QUERY_KEY, "detail", id],
    queryFn: () => (id ? frameDetail(id) : Promise.resolve(null)),
    enabled: id !== null,
  });
}
