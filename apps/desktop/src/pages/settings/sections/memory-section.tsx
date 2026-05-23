import { useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FolderOpen, Loader2, RefreshCcw, Trash2 } from "lucide-react";

import type { Note, NoteScope, NoteSourceType, NoteStatus } from "@corivo/shared-types";
import { Button } from "@repo/ui/components/button";
import { Textarea } from "@repo/ui/components/textarea";

import {
  createNote,
  deleteNote,
  getAutoPersona,
  listNotes,
  openBackgroundTaskLogsDir,
  regenerateAutoPersona,
  updateNote,
} from "@/lib/tauri";

import { FieldGroup, SectionHeader, SettingsSkeleton } from "./settings-shared";

const SCOPE_LABEL: Record<NoteScope, string> = {
  global: "全局",
  project: "项目",
  session: "会话",
};

const SOURCE_LABEL: Record<NoteSourceType, string> = {
  user_explicit: "用户明确",
  agent_inferred: "Agent 推断",
};

const STATUS_LABEL: Record<NoteStatus, string> = {
  active: "生效",
  suggested: "候选",
  superseded: "已替换",
  contradicted: "已被否定",
  archived: "已归档",
};

/**
 * Memory settings tab (memory-system-spec §3 Step 1a).
 *
 * Two areas: live persistent notes (scope=global + status=active) at
 * the top — these are what gets injected into every prompt — and the
 * inferred-candidates list below, where the user promotes / discards
 * agent-suggested rows.
 */
export function MemorySection() {
  const qc = useQueryClient();
  const { data, isLoading } = useQuery({
    queryKey: ["notes-all"],
    queryFn: () => listNotes(),
    refetchOnWindowFocus: false,
  });

  const create = useMutation({
    mutationFn: createNote,
    onSuccess: () => qc.invalidateQueries({ queryKey: ["notes-all"] }),
  });
  const remove = useMutation({
    mutationFn: deleteNote,
    onSuccess: () => qc.invalidateQueries({ queryKey: ["notes-all"] }),
  });
  const updateMutation = useMutation({
    mutationFn: updateNote,
    onSuccess: () => qc.invalidateQueries({ queryKey: ["notes-all"] }),
  });

  if (isLoading || !data) {
    return <SettingsSkeleton />;
  }

  const persistent = data.filter(
    (n) =>
      n.scope === "global" &&
      n.source_type === "user_explicit" &&
      n.status === "active",
  );
  const suggested = data.filter((n) => n.status === "suggested");
  const others = data.filter(
    (n) => !persistent.includes(n) && !suggested.includes(n),
  );

  return (
    <div className="max-w-2xl space-y-6">
      <SectionHeader
        title="记忆"
        description="Corivo 长期记住的偏好与事实。生效项每次对话都会注入到模型 prompt 中。"
      />

      <NewNoteForm
        disabled={create.isPending}
        onSubmit={(content) => create.mutate({ content, scope: "global" })}
      />

      <FieldGroup title="生效中">
        {persistent.length === 0 ? (
          <EmptyHint text="还没有生效的记忆。直接添加，或在对话中告诉 Corivo 记住某件事。" />
        ) : (
          <NoteList
            notes={persistent}
            onDelete={(id) => remove.mutate(id)}
            disabled={remove.isPending}
          />
        )}
      </FieldGroup>

      {suggested.length > 0 ? (
        <FieldGroup title="候选 (Agent 推断)">
          <p className="text-[11.5px] text-muted-foreground">
            这些是 Corivo 在对话中觉得可能值得记住的偏好。确认后才会注入 prompt。
          </p>
          <NoteList
            notes={suggested}
            onDelete={(id) => remove.mutate(id)}
            disabled={remove.isPending || updateMutation.isPending}
            extraActions={(note) => (
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={updateMutation.isPending}
                onClick={() =>
                  updateMutation.mutate({ id: note.id, status: "active" })
                }
              >
                确认记住
              </Button>
            )}
          />
        </FieldGroup>
      ) : null}

      {others.length > 0 ? (
        <FieldGroup title="其他">
          <NoteList
            notes={others}
            onDelete={(id) => remove.mutate(id)}
            disabled={remove.isPending}
          />
        </FieldGroup>
      ) : null}

      <PersonaCard />

      <BackgroundTaskLogsCard />
    </div>
  );
}

function BackgroundTaskLogsCard() {
  return (
    <FieldGroup title="后台任务日志">
      <div className="flex items-center justify-between text-[11.5px] text-muted-foreground">
        <span>
          每次记忆 / 画像后台任务会写一份独立日志，记录系统提示、工具调用、结果和耗时。
        </span>
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={() => void openBackgroundTaskLogsDir()}
        >
          <FolderOpen className="mr-1 h-3 w-3" />
          打开日志目录
        </Button>
      </div>
    </FieldGroup>
  );
}

function PersonaCard() {
  const qc = useQueryClient();
  const { data: persona, isLoading } = useQuery({
    queryKey: ["auto-persona"],
    queryFn: getAutoPersona,
    refetchOnWindowFocus: false,
  });
  const regen = useMutation({
    mutationFn: regenerateAutoPersona,
    onSuccess: () => {
      // The actual file gets written by the background task; poll
      // once after a short delay so a freshly regenerated persona
      // shows up without forcing the user to switch tabs.
      setTimeout(
        () => qc.invalidateQueries({ queryKey: ["auto-persona"] }),
        5_000,
      );
    },
  });

  return (
    <FieldGroup title="画像 (auto-persona.md)">
      <div className="flex items-center justify-between text-[11.5px] text-muted-foreground">
        <span>
          每日由 Corivo 后台自动重新生成。如与上方"生效中"的偏好冲突，以偏好为准。
        </span>
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={() => regen.mutate()}
          disabled={regen.isPending}
        >
          {regen.isPending ? (
            <Loader2 className="mr-1 h-3 w-3 animate-spin" />
          ) : (
            <RefreshCcw className="mr-1 h-3 w-3" />
          )}
          立刻重新生成
        </Button>
      </div>
      {isLoading ? (
        <div className="rounded-md border border-dashed border-border/60 p-4 text-xs text-muted-foreground">
          加载中…
        </div>
      ) : !persona || persona.trim().length === 0 ? (
        <div className="rounded-md border border-dashed border-border/60 p-4 text-xs text-muted-foreground">
          画像还没有生成。Corivo 在你使用一段时间后会自动产生第一版；也可以现在手动触发。
        </div>
      ) : (
        <pre className="max-h-[420px] overflow-auto rounded-md border border-border/40 bg-muted/30 p-3 text-[12px] leading-relaxed whitespace-pre-wrap">
          {persona}
        </pre>
      )}
    </FieldGroup>
  );
}

function NewNoteForm({
  disabled,
  onSubmit,
}: {
  disabled: boolean;
  onSubmit: (content: string) => void;
}) {
  const [text, setText] = useState("");
  const submit = (event: FormEvent) => {
    event.preventDefault();
    const trimmed = text.trim();
    if (!trimmed) return;
    onSubmit(trimmed);
    setText("");
  };
  return (
    <form onSubmit={submit} className="flex flex-col gap-2">
      <Textarea
        value={text}
        onChange={(event) => setText(event.target.value)}
        placeholder="例如：以后默认用中文回我"
        rows={2}
        disabled={disabled}
      />
      <div className="flex items-center justify-end gap-2">
        <Button type="submit" size="sm" disabled={disabled || !text.trim()}>
          {disabled ? (
            <Loader2 className="mr-1 h-3 w-3 animate-spin" />
          ) : null}
          添加到记忆
        </Button>
      </div>
    </form>
  );
}

function NoteList({
  notes,
  onDelete,
  disabled,
  extraActions,
}: {
  notes: Note[];
  onDelete: (id: string) => void;
  disabled: boolean;
  extraActions?: (note: Note) => React.ReactNode;
}) {
  return (
    <ul className="divide-y divide-border/40 rounded-md border border-border/40">
      {notes.map((note) => (
        <li key={note.id} className="flex items-start gap-3 p-3">
          <div className="min-w-0 flex-1 space-y-1">
            <div className="text-[13px] leading-snug text-foreground">
              {note.content}
            </div>
            <div className="flex flex-wrap items-center gap-1.5 text-[10.5px] text-muted-foreground">
              <Badge>{SCOPE_LABEL[note.scope]}</Badge>
              <Badge>{SOURCE_LABEL[note.source_type]}</Badge>
              <Badge>{STATUS_LABEL[note.status]}</Badge>
              <span>·</span>
              <span>{new Date(note.created_at).toLocaleString()}</span>
            </div>
          </div>
          <div className="flex items-center gap-1.5">
            {extraActions?.(note)}
            <Button
              type="button"
              variant="ghost"
              size="icon"
              onClick={() => onDelete(note.id)}
              disabled={disabled}
              aria-label="删除"
            >
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </div>
        </li>
      ))}
    </ul>
  );
}

function Badge({ children }: { children: React.ReactNode }) {
  return (
    <span className="rounded-sm bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
      {children}
    </span>
  );
}

function EmptyHint({ text }: { text: string }) {
  return (
    <div className="rounded-md border border-dashed border-border/60 p-4 text-xs text-muted-foreground">
      {text}
    </div>
  );
}
