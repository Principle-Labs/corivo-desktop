import {
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

import {
  dataDeleteRange,
  getCaptureStatus,
  pauseCapture,
  resumeCapture,
  startCapture,
  stopCapture,
} from "@/lib/tauri";
import { FRAMES_QUERY_KEY } from "@/hooks/use-frames";

export const CAPTURE_STATUS_QUERY_KEY = ["capture", "status"] as const;

export function useCaptureStatus() {
  return useQuery({
    queryKey: CAPTURE_STATUS_QUERY_KEY,
    queryFn: getCaptureStatus,
    refetchInterval: 3000,
  });
}

export function useStartCapture() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => startCapture(),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: CAPTURE_STATUS_QUERY_KEY });
      qc.invalidateQueries({ queryKey: FRAMES_QUERY_KEY });
    },
  });
}

export function useStopCapture() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => stopCapture(),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: CAPTURE_STATUS_QUERY_KEY });
    },
  });
}

export function usePauseCapture() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (seconds: number) => pauseCapture(seconds),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: CAPTURE_STATUS_QUERY_KEY });
    },
  });
}

export function useResumeCapture() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => resumeCapture(),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: CAPTURE_STATUS_QUERY_KEY });
      qc.invalidateQueries({ queryKey: FRAMES_QUERY_KEY });
    },
  });
}

/**
 * Granular delete: drop every frame captured in the trailing
 * `secondsBack` seconds + their on-disk screenshots. Backs the
 * "delete last 5 min / 15 min / custom…" entries in the privacy popover.
 * Frame data evaporates from /ask and /timeline on success, so we
 * invalidate the frames query immediately.
 */
export function useDataDeleteRange() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (secondsBack: number) => dataDeleteRange(secondsBack),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: FRAMES_QUERY_KEY });
    },
  });
}
