import { useEffect, useState } from "react";
import { api, type AppConfig, type ModelOptions } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { useI18n } from "@/i18n/context";
import { PageLoading } from "@/components/page-loading";
import { useToast } from "@/components/ui/toast";

export function MappingPage() {
  const { t } = useI18n();
  const { toast } = useToast();
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [models, setModels] = useState<ModelOptions | null>(null);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    Promise.all([api.getConfig(), api.listModelOptions()])
      .then(([cfg, opts]) => {
        setConfig(cfg);
        setModels(opts);
        setFrom(opts.codex[0] ?? "gpt-5.6");
        setTo(cfg.defaultModel || opts.grokText[0] || "grok-4.5");
      })
      .catch((e) => setError(String(e)));
  }, []);

  if (!config && !error) {
    return <PageLoading />;
  }

  if (!config) {
    return (
      <div className="space-y-3">
        <div className="text-sm text-red-600">
          {t.common.loadFailed}
          {error ? `：${error}` : ""}
        </div>
        <Button
          variant="outline"
          onClick={() => {
            setError("");
            Promise.all([api.getConfig(), api.listModelOptions()])
              .then(([cfg, opts]) => {
                setConfig(cfg);
                setModels(opts);
              })
              .catch((e) => setError(String(e)));
          }}
        >
          {t.common.retry}
        </Button>
      </div>
    );
  }

  // Include existing mapping keys so legacy entries stay selectable.
  const codexList = ensureOption(
    [...(models?.codex ?? []), ...Object.keys(config.modelMappings)],
    from
  );
  const grokTextList = ensureOption(
    [...(models?.grokText ?? []), ...Object.values(config.modelMappings), config.defaultModel],
    to
  );

  async function save(next: AppConfig) {
    try {
      const saved = await api.updateConfig(next);
      setConfig(saved);
      toast(t.common.saved, "success");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  return (
    <div className="space-y-4 overflow-y-auto">
      <h1 className="text-xl font-semibold tracking-tight">{t.mapping.title}</h1>

      <Card>
        <CardHeader className="py-3">
          <CardTitle className="text-base">{t.mapping.defaultText}</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3 sm:flex-row sm:items-end">
          <div className="min-w-0 flex-1">
            <Label>{t.settings.defaultText}</Label>
            <Select
              value={config.defaultModel}
              onChange={(e) => setConfig({ ...config, defaultModel: e.target.value })}
            >
              {ensureOption(grokTextList, config.defaultModel).map((id) => (
                <option key={id} value={id}>
                  {id}
                </option>
              ))}
            </Select>
          </div>
          <Button onClick={() => save(config)}>{t.common.save}</Button>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t.mapping.mappings}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-3 md:grid-cols-[1fr_1fr_auto]">
            <div>
              <Label>{t.mapping.from}</Label>
              <Select value={from} onChange={(e) => setFrom(e.target.value)}>
                {codexList.map((id) => (
                  <option key={id} value={id}>
                    {id}
                  </option>
                ))}
              </Select>
            </div>
            <div>
              <Label>{t.mapping.to}</Label>
              <Select value={to} onChange={(e) => setTo(e.target.value)}>
                {grokTextList.map((id) => (
                  <option key={id} value={id}>
                    {id}
                  </option>
                ))}
              </Select>
            </div>
            <div className="flex items-end">
              <Button
                onClick={() => {
                  if (!from || !to) return;
                  const lower = to.toLowerCase();
                  if (
                    lower.includes("image") ||
                    lower.includes("video") ||
                    lower.includes("imagine")
                  ) {
                    toast(t.mapping.targetMustBeText, "warning");
                    return;
                  }
                  const modelMappings = { ...config.modelMappings, [from]: to };
                  save({ ...config, modelMappings });
                }}
              >
                {t.mapping.addUpdate}
              </Button>
            </div>
          </div>

          <div className="space-y-2">
            {Object.entries(config.modelMappings).map(([k, v]) => (
              <div
                key={k}
                className="flex items-center justify-between rounded-md border border-neutral-200 px-3 py-2 text-sm"
              >
                <div>
                  <span className="font-medium">{k}</span>
                  <span className="mx-2 text-neutral-400">→</span>
                  <span>{v}</span>
                </div>
                <Button
                  variant="ghost"
                  onClick={() => {
                    const modelMappings = { ...config.modelMappings };
                    delete modelMappings[k];
                    save({ ...config, modelMappings });
                  }}
                >
                  {t.common.remove}
                </Button>
              </div>
            ))}

          </div>
        </CardContent>
      </Card>
      {error ? <div className="text-sm text-red-600">{error}</div> : null}
    </div>
  );
}

function ensureOption(list: string[] | undefined, current: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const id of [...(list ?? []), current]) {
    if (!id || seen.has(id)) continue;
    seen.add(id);
    out.push(id);
  }
  return out;
}
