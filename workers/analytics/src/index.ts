interface Env {
  DB: D1Database;
}

const corsHeaders = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
};

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    if (request.method === "OPTIONS") {
      return new Response(null, { status: 204, headers: corsHeaders });
    }

    const url = new URL(request.url);

    if (url.pathname === "/ping" && request.method === "POST") {
      return handlePing(request, env);
    }

    if (url.pathname === "/stats" && request.method === "GET") {
      return handleStats(env);
    }

    return new Response("Not Found", { status: 404, headers: corsHeaders });
  },
} satisfies ExportedHandler<Env>;

async function handlePing(request: Request, env: Env): Promise<Response> {
  try {
    const body = await request.json<{ id?: string; v?: string; platform?: string }>();
    const { id, v, platform } = body;

    if (!id || !v || !platform) {
      return new Response("Bad Request", { status: 400, headers: corsHeaders });
    }

    await env.DB.prepare(
      "INSERT OR IGNORE INTO pings (install_id, version, platform) VALUES (?, ?, ?)"
    )
      .bind(id, v, platform)
      .run();

    return new Response(null, { status: 204, headers: corsHeaders });
  } catch {
    return new Response("Bad Request", { status: 400, headers: corsHeaders });
  }
}

async function handleStats(env: Env): Promise<Response> {
  const today = new Date().toISOString().slice(0, 10);
  const sevenDaysAgo = new Date(Date.now() - 7 * 86400000).toISOString().slice(0, 10);

  const [totalResult, dauResult, versionsResult] = await Promise.all([
    env.DB.prepare("SELECT COUNT(DISTINCT install_id) AS cnt FROM pings").first<{ cnt: number }>(),
    env.DB.prepare("SELECT COUNT(DISTINCT install_id) AS cnt FROM pings WHERE date = ?")
      .bind(today)
      .first<{ cnt: number }>(),
    env.DB.prepare(
      "SELECT version, COUNT(DISTINCT install_id) AS count FROM pings WHERE date >= ? GROUP BY version ORDER BY count DESC"
    )
      .bind(sevenDaysAgo)
      .all<{ version: string; count: number }>(),
  ]);

  const stats = {
    totalInstalls: totalResult?.cnt ?? 0,
    dau: dauResult?.cnt ?? 0,
    versions: versionsResult.results,
  };

  return new Response(JSON.stringify(stats), {
    headers: { ...corsHeaders, "Content-Type": "application/json" },
  });
}
