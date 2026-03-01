<script lang="ts">
	import { page } from '$app/stores';

	interface SentinelLogEntry {
		id: string;
		sentinel_id: string;
		message: string;
		created_at: string;
	}

	interface SentinelInfo {
		id: string;
		name: string;
		connected: boolean;
		last_connected_at: string | null;
		created_at: string;
	}

	let sentinelId = $derived($page.params.id);
	let sentinel: SentinelInfo | null = $state(null);
	let logs: SentinelLogEntry[] = $state([]);
	let loading = $state(true);

	// WebSocket
	let ws: WebSocket | null = null;
	let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

	function formatDate(iso: string): string {
		return new Date(iso).toLocaleDateString(undefined, {
			month: 'short',
			day: 'numeric',
			hour: '2-digit',
			minute: '2-digit',
			second: '2-digit'
		});
	}

	function formatTime(iso: string): string {
		return new Date(iso).toLocaleTimeString(undefined, {
			hour: '2-digit',
			minute: '2-digit',
			second: '2-digit'
		});
	}

	async function loadSentinelData() {
		loading = true;
		try {
			const [sentinelsRes, logsRes] = await Promise.all([
				fetch('/api/sentinel/sentinels'),
				fetch(`/api/sentinel/sentinels/${sentinelId}/logs?limit=500`)
			]);

			if (sentinelsRes.ok) {
				const all: SentinelInfo[] = await sentinelsRes.json();
				sentinel = all.find((s) => s.id === sentinelId) ?? null;
			}

			if (logsRes.ok) {
				logs = await logsRes.json();
			}
		} catch {
			// ignore
		} finally {
			loading = false;
		}
	}

	function connectWebSocket() {
		if (ws) return;

		const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
		const url = `${proto}//${location.host}/api/ws`;
		const socket = new WebSocket(url);

		socket.addEventListener('open', () => {
			ws = socket;
		});

		socket.addEventListener('message', (ev) => {
			try {
				const msg = JSON.parse(ev.data);
				handleWsMessage(msg);
			} catch {
				// ignore
			}
		});

		socket.addEventListener('close', () => {
			ws = null;
			scheduleReconnect();
		});

		socket.addEventListener('error', () => {
			socket.close();
		});
	}

	function scheduleReconnect() {
		if (reconnectTimer) return;
		reconnectTimer = setTimeout(() => {
			reconnectTimer = null;
			connectWebSocket();
		}, 3000);
	}

	function handleWsMessage(msg: { type: string; data: Record<string, unknown> }) {
		switch (msg.type) {
			case 'sentinel_log': {
				if ((msg.data.sentinel_id as string) !== sentinelId) break;
				const newLog: SentinelLogEntry = {
					id: crypto.randomUUID(),
					sentinel_id: msg.data.sentinel_id as string,
					message: msg.data.message as string,
					created_at: msg.data.created_at as string
				};
				logs = [newLog, ...logs];
				break;
			}
			case 'sentinel_connected': {
				if ((msg.data.id as string) !== sentinelId) break;
				if (sentinel) {
					sentinel = { ...sentinel, connected: true, last_connected_at: new Date().toISOString() };
				}
				break;
			}
			case 'sentinel_disconnected': {
				if ((msg.data.id as string) !== sentinelId) break;
				if (sentinel) {
					sentinel = { ...sentinel, connected: false };
				}
				break;
			}
		}
	}

	$effect(() => {
		loadSentinelData();
		connectWebSocket();

		return () => {
			if (reconnectTimer) clearTimeout(reconnectTimer);
			if (ws) ws.close();
		};
	});
</script>

<svelte:head>
	<title>{sentinel?.name ?? 'Sentinel'} - Panopticon</title>
</svelte:head>

<main class="flex flex-1 justify-center p-6 lg:pt-10">
	<div class="w-full max-w-3xl space-y-6">
		<!-- Back link + header -->
		<div class="flex items-center gap-4">
			<a href="/" class="text-sm text-surface-500 hover:text-surface-300">&larr; Back</a>
		</div>

		{#if loading}
			<div class="card preset-filled-surface-900 p-6">
				<p class="text-sm text-surface-400 animate-pulse">Loading sentinel...</p>
			</div>
		{:else if !sentinel}
			<div class="card preset-filled-surface-900 p-6">
				<p class="text-sm text-error-400">Sentinel not found.</p>
			</div>
		{:else}
			<!-- Sentinel info -->
			<div class="card preset-filled-surface-900 space-y-3 p-6">
				<div class="flex items-center justify-between">
					<div class="flex items-center gap-3">
						<div
							class="h-2.5 w-2.5 rounded-full {sentinel.connected
								? 'bg-success-500'
								: 'bg-surface-600'}"
						></div>
						<h1 class="h4">{sentinel.name}</h1>
					</div>
					<span
						class="inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium {sentinel.connected
							? 'bg-success-500/15 text-success-400'
							: 'bg-surface-700 text-surface-400'}"
					>
						{sentinel.connected ? 'Connected' : 'Disconnected'}
					</span>
				</div>
				{#if sentinel.last_connected_at}
					<p class="text-xs text-surface-500">
						Last connected: {formatDate(sentinel.last_connected_at)}
					</p>
				{/if}
			</div>

			<!-- Logs -->
			<div class="card preset-filled-surface-900 space-y-4 p-6">
				<h2 class="h5">Logs</h2>
				{#if logs.length === 0}
					<p class="text-sm text-surface-400">No logs yet.</p>
				{:else}
					<div class="max-h-[600px] overflow-y-auto space-y-0.5">
						{#each logs as entry (entry.id)}
							<div class="flex gap-3 rounded px-2 py-1 hover:bg-surface-800">
								<span class="flex-shrink-0 text-xs text-surface-500 font-mono">
									{formatTime(entry.created_at)}
								</span>
								<span class="text-xs text-surface-300 font-mono break-all">
									{entry.message}
								</span>
							</div>
						{/each}
					</div>
				{/if}
			</div>
		{/if}
	</div>
</main>
