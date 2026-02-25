<script lang="ts">
	interface UtecStatus {
		authenticated: boolean;
		user_name: string | null;
		expires_at: string | null;
	}

	interface DeviceInfo {
		id: string;
		name: string;
		lock_state: string | null;
		battery_level: number | null;
		online: boolean;
	}

	interface AccessCard {
		id: string;
		tag_id: string;
		label: string | null;
		created_at: string;
	}

	interface ScanLogEntry {
		id: string;
		tag_id: string;
		action: string;
		created_at: string;
	}

	let utecStatus: UtecStatus | null = $state(null);
	let devices: DeviceInfo[] = $state([]);
	let loading = $state(true);
	let devicesLoading = $state(false);
	let actionInFlight: Record<string, boolean> = $state({});
	let error: string | null = $state(null);

	// Access control state
	let sentinelMode: string = $state('guard');
	let modeLoading = $state(false);
	let cards: AccessCard[] = $state([]);
	let scanLog: ScanLogEntry[] = $state([]);

	async function checkUtec() {
		try {
			const res = await fetch('/auth/status');
			utecStatus = await res.json();
		} catch {
			utecStatus = null;
		} finally {
			loading = false;
		}
	}

	async function loadDevices() {
		devicesLoading = true;
		error = null;
		try {
			const res = await fetch('/api/devices');
			if (res.status === 503) {
				devices = [];
				return;
			}
			if (!res.ok) throw new Error('Failed to load devices');
			devices = await res.json();
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load devices';
		} finally {
			devicesLoading = false;
		}
	}

	async function toggleLock(device: DeviceInfo) {
		const action = device.lock_state === 'locked' ? 'unlock' : 'lock';
		actionInFlight = { ...actionInFlight, [device.id]: true };
		try {
			const res = await fetch(`/api/devices/${device.id}/${action}`, { method: 'POST' });
			if (!res.ok) throw new Error(`Failed to ${action}`);
			const result = await res.json();
			devices = devices.map((d) =>
				d.id === device.id ? { ...d, lock_state: result.lock_state } : d
			);
		} catch (e) {
			error = e instanceof Error ? e.message : `Failed to ${action}`;
		} finally {
			actionInFlight = { ...actionInFlight, [device.id]: false };
		}
	}

	async function disconnectUtec() {
		try {
			await fetch('/auth/logout', { method: 'DELETE' });
			utecStatus = { authenticated: false, user_name: null, expires_at: null };
			devices = [];
		} catch {
			// ignore
		}
	}

	async function handleLogout() {
		await fetch('/api/auth/logout', { method: 'POST' });
		window.location.href = '/login';
	}

	async function loadSentinelMode() {
		try {
			const res = await fetch('/api/sentinel/mode');
			if (res.ok) {
				const data = await res.json();
				sentinelMode = data.mode;
			}
		} catch {
			// ignore
		}
	}

	async function toggleMode() {
		const newMode = sentinelMode === 'guard' ? 'enroll' : 'guard';
		modeLoading = true;
		try {
			const res = await fetch('/api/sentinel/mode', {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ mode: newMode })
			});
			if (res.ok) {
				const data = await res.json();
				sentinelMode = data.mode;
			}
		} catch {
			// ignore
		} finally {
			modeLoading = false;
		}
	}

	async function loadCards() {
		try {
			const res = await fetch('/api/sentinel/cards');
			if (res.ok) cards = await res.json();
		} catch {
			// ignore
		}
	}

	async function removeCard(id: string) {
		try {
			const res = await fetch(`/api/sentinel/cards/${id}`, { method: 'DELETE' });
			if (res.ok) cards = cards.filter((c) => c.id !== id);
		} catch {
			// ignore
		}
	}

	async function loadScanLog() {
		try {
			const res = await fetch('/api/sentinel/scan-log');
			if (res.ok) scanLog = await res.json();
		} catch {
			// ignore
		}
	}

	function formatDate(iso: string): string {
		return new Date(iso).toLocaleDateString(undefined, {
			month: 'short',
			day: 'numeric',
			hour: '2-digit',
			minute: '2-digit'
		});
	}

	$effect(() => {
		checkUtec();
		loadSentinelMode();
		loadCards();
		loadScanLog();
	});

	$effect(() => {
		if (utecStatus?.authenticated) {
			loadDevices();
		}
	});
</script>

<svelte:head>
	<title>Panopticon</title>
</svelte:head>

<main class="flex flex-1 items-center justify-center p-6">
	<div class="w-full max-w-md space-y-6">
		<div class="flex items-center justify-between">
			<h1 class="h2">Panopticon</h1>
			<button
				class="text-sm text-surface-500 hover:text-surface-300 cursor-pointer"
				onclick={handleLogout}
			>
				Sign out
			</button>
		</div>

		{#if loading}
			<div class="card preset-filled-surface-900 p-6">
				<p class="text-sm text-surface-400 animate-pulse">Checking connection...</p>
			</div>
		{:else if !utecStatus?.authenticated}
			<div class="card preset-filled-surface-900 space-y-5 p-6">
				<h2 class="h5">U-Tec Smart Lock</h2>
				<p class="text-sm text-surface-400">
					Connect your U-Tec account to manage your smart locks.
				</p>
				<a href="/auth/login" class="btn btn-base preset-filled-primary-500 w-full">
					Connect U-Tec Account
				</a>
			</div>
		{:else}
			<!-- Connected header -->
			<div class="card preset-filled-surface-900 space-y-4 p-6">
				<div class="flex items-center justify-between">
					<div class="flex items-center gap-3">
						<div class="h-2 w-2 rounded-full bg-success-500"></div>
						<div>
							<p class="text-sm font-medium text-surface-200">U-Tec Connected</p>
							<p class="text-xs text-surface-400">
								{utecStatus.user_name ?? 'Unknown user'}
							</p>
						</div>
					</div>
					<button
						class="text-xs text-surface-500 hover:text-surface-300 cursor-pointer"
						onclick={disconnectUtec}
					>
						Disconnect
					</button>
				</div>
			</div>

			<!-- Devices -->
			{#if devicesLoading}
				<div class="card preset-filled-surface-900 p-6">
					<p class="text-sm text-surface-400 animate-pulse">Loading locks...</p>
				</div>
			{:else if error}
				<div class="card preset-filled-surface-900 space-y-3 p-6">
					<p class="text-sm text-error-400">{error}</p>
					<button class="btn btn-sm preset-outlined-surface-500" onclick={loadDevices}>
						Retry
					</button>
				</div>
			{:else if devices.length === 0}
				<div class="card preset-filled-surface-900 p-6">
					<p class="text-sm text-surface-400">No locks found on your account.</p>
				</div>
			{:else}
				{#each devices as device (device.id)}
					<div class="card preset-filled-surface-900 space-y-4 p-6">
						<div class="flex items-center justify-between">
							<h3 class="text-base font-medium text-surface-100">{device.name}</h3>
							<div class="flex items-center gap-2">
								{#if !device.online}
									<span class="text-xs text-surface-500">Offline</span>
								{/if}
								<div
									class="h-2 w-2 rounded-full {device.online
										? 'bg-success-500'
										: 'bg-surface-600'}"
								></div>
							</div>
						</div>

						<div class="flex items-center justify-between">
							<div class="flex items-center gap-3">
								<!-- Lock state indicator -->
								{#if device.lock_state === 'locked'}
									<div
										class="flex h-10 w-10 items-center justify-center rounded-full bg-success-500/15"
									>
										<svg
											class="h-5 w-5 text-success-500"
											fill="none"
											viewBox="0 0 24 24"
											stroke="currentColor"
											stroke-width="2"
										>
											<path
												stroke-linecap="round"
												stroke-linejoin="round"
												d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"
											/>
										</svg>
									</div>
									<span class="text-sm font-medium text-success-400">Locked</span>
								{:else if device.lock_state === 'unlocked'}
									<div
										class="flex h-10 w-10 items-center justify-center rounded-full bg-warning-500/15"
									>
										<svg
											class="h-5 w-5 text-warning-500"
											fill="none"
											viewBox="0 0 24 24"
											stroke="currentColor"
											stroke-width="2"
										>
											<path
												stroke-linecap="round"
												stroke-linejoin="round"
												d="M8 11V7a4 4 0 118 0m-4 8v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2z"
											/>
										</svg>
									</div>
									<span class="text-sm font-medium text-warning-400">Unlocked</span>
								{:else}
									<div
										class="flex h-10 w-10 items-center justify-center rounded-full bg-surface-700"
									>
										<svg
											class="h-5 w-5 text-surface-400"
											fill="none"
											viewBox="0 0 24 24"
											stroke="currentColor"
											stroke-width="2"
										>
											<path
												stroke-linecap="round"
												stroke-linejoin="round"
												d="M8.228 9c.549-1.165 2.03-2 3.772-2 2.21 0 4 1.343 4 3 0 1.4-1.278 2.575-3.006 2.907-.542.104-.994.54-.994 1.093m0 3h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
											/>
										</svg>
									</div>
									<span class="text-sm text-surface-400">Unknown</span>
								{/if}
							</div>

							<!-- Battery -->
							{#if device.battery_level != null}
								<div class="flex items-center gap-1 text-xs text-surface-400">
									<svg
										class="h-4 w-4"
										fill="none"
										viewBox="0 0 24 24"
										stroke="currentColor"
										stroke-width="2"
									>
										<path
											stroke-linecap="round"
											stroke-linejoin="round"
											d="M3 7h14a2 2 0 012 2v6a2 2 0 01-2 2H3a2 2 0 01-2-2V9a2 2 0 012-2zm18 3v4"
										/>
									</svg>
									{device.battery_level}%
								</div>
							{/if}
						</div>

						<!-- Lock/Unlock button -->
						<button
							class="btn btn-base w-full {device.lock_state === 'locked'
								? 'preset-outlined-warning-500'
								: 'preset-outlined-success-500'}"
							disabled={actionInFlight[device.id] || !device.online}
							onclick={() => toggleLock(device)}
						>
							{#if actionInFlight[device.id]}
								<span class="animate-pulse">
									{device.lock_state === 'locked' ? 'Unlocking...' : 'Locking...'}
								</span>
							{:else if !device.online}
								Offline
							{:else if device.lock_state === 'locked'}
								Unlock
							{:else}
								Lock
							{/if}
						</button>
					</div>
				{/each}
			{/if}
		{/if}

		<!-- Access Control -->
		<div class="card preset-filled-surface-900 space-y-4 p-6">
			<div class="flex items-center justify-between">
				<h2 class="h5">Access Control</h2>
				<button
					class="btn btn-sm {sentinelMode === 'enroll'
						? 'preset-filled-warning-500'
						: 'preset-outlined-surface-500'}"
					disabled={modeLoading}
					onclick={toggleMode}
				>
					{#if modeLoading}
						<span class="animate-pulse">Switching...</span>
					{:else}
						{sentinelMode === 'guard' ? 'Guard Mode' : 'Enroll Mode'}
					{/if}
				</button>
			</div>
			{#if sentinelMode === 'enroll'}
				<div class="rounded-md bg-warning-500/15 px-3 py-2">
					<p class="text-xs text-warning-400">
						Enroll mode active — scan a card to register it. Switch back to guard mode when done.
					</p>
				</div>
			{/if}
		</div>

		<!-- Enrolled Cards -->
		<div class="card preset-filled-surface-900 space-y-4 p-6">
			<h2 class="h5">Enrolled Cards</h2>
			{#if cards.length === 0}
				<p class="text-sm text-surface-400">No cards enrolled yet.</p>
			{:else}
				<div class="space-y-2">
					{#each cards as card (card.id)}
						<div class="flex items-center justify-between rounded-md bg-surface-800 px-3 py-2">
							<div>
								<p class="font-mono text-sm text-surface-200">{card.tag_id}</p>
								<p class="text-xs text-surface-500">{formatDate(card.created_at)}</p>
							</div>
							<button
								class="text-xs text-error-400 hover:text-error-300 cursor-pointer"
								onclick={() => removeCard(card.id)}
							>
								Remove
							</button>
						</div>
					{/each}
				</div>
			{/if}
		</div>

		<!-- Recent Scans -->
		<div class="card preset-filled-surface-900 space-y-4 p-6">
			<h2 class="h5">Recent Scans</h2>
			{#if scanLog.length === 0}
				<p class="text-sm text-surface-400">No scans recorded yet.</p>
			{:else}
				<div class="space-y-2">
					{#each scanLog.slice(0, 10) as entry (entry.id)}
						<div class="flex items-center gap-3 rounded-md bg-surface-800 px-3 py-2">
							<div
								class="h-2 w-2 flex-shrink-0 rounded-full {entry.action === 'granted'
									? 'bg-success-500'
									: entry.action === 'denied'
										? 'bg-error-500'
										: 'bg-primary-500'}"
							></div>
							<div class="flex-1 min-w-0">
								<p class="font-mono text-sm text-surface-200 truncate">{entry.tag_id}</p>
							</div>
							<div class="flex-shrink-0 text-right">
								<p class="text-xs capitalize text-surface-300">{entry.action}</p>
								<p class="text-xs text-surface-500">{formatDate(entry.created_at)}</p>
							</div>
						</div>
					{/each}
				</div>
			{/if}
		</div>
	</div>
</main>
