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

	interface LockUser {
		id: number;
		name: string;
		type: number;
		status: number;
		sync_status: number;
	}

	interface ScanLogEntry {
		id: string;
		tag_id: string;
		action: string;
		created_at: string;
	}

	interface PendingUser {
		id: string;
		email: string;
		email_confirmed: boolean;
		created_at: string;
	}

	interface SentinelInfo {
		id: string;
		name: string;
		connected: boolean;
		last_connected_at: string | null;
		created_at: string;
	}

	let utecStatus: UtecStatus | null = $state(null);
	let isUtecAuthenticated = $derived(utecStatus?.authenticated ?? false);
	let devices: DeviceInfo[] = $state([]);
	let loading = $state(true);
	let devicesLoading = $state(false);
	let actionInFlight: Record<string, boolean> = $state({});
	let pendingAction: Record<string, 'locking' | 'unlocking'> = $state({});
	let lockUsers: Record<string, LockUser[]> = $state({});
	let lockUsersLoading: Record<string, boolean> = $state({});
	let error: string | null = $state(null);

	// Pending users (admin)
	let pendingUsers: PendingUser[] = $state([]);
	let pendingUsersError: string | null = $state(null);

	// Access control state
	let sentinelMode: string = $state('guard');
	let modeLoading = $state(false);
	let cards: AccessCard[] = $state([]);
	let scanLog: ScanLogEntry[] = $state([]);

	// Sentinels
	let sentinels: SentinelInfo[] = $state([]);

	// Current user
	let currentUserEmail: string | null = $state(null);

	// Notification state
	let browserNotifications: boolean = $state(
		typeof Notification !== 'undefined' && Notification.permission === 'granted'
	);
	let emailNotifications: boolean = $state(false);
	let pushNotifications: boolean = $state(false);
	let pushLoading: boolean = $state(false);

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

	async function loadDevices(): Promise<DeviceInfo[]> {
		devicesLoading = true;
		error = null;
		try {
			const res = await fetch('/api/devices');
			if (res.status === 503) {
				devices = [];
				return [];
			}
			if (!res.ok) throw new Error('Failed to load devices');
			const loaded: DeviceInfo[] = await res.json();
			devices = loaded;
			return loaded;
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load devices';
			return [];
		} finally {
			devicesLoading = false;
		}
	}

	async function loadLockUsers(deviceId: string) {
		lockUsersLoading = { ...lockUsersLoading, [deviceId]: true };
		try {
			const res = await fetch(`/api/devices/${deviceId}/users`);
			if (res.ok) {
				lockUsers = { ...lockUsers, [deviceId]: await res.json() };
			} else {
				console.error(`Failed to load lock users for ${deviceId}: ${res.status}`);
			}
		} catch (e) {
			console.error(`Failed to load lock users for ${deviceId}:`, e);
		} finally {
			lockUsersLoading = { ...lockUsersLoading, [deviceId]: false };
		}
	}

	async function toggleLock(device: DeviceInfo) {
		const action = device.lock_state === 'locked' ? 'unlock' : 'lock';
		actionInFlight = { ...actionInFlight, [device.id]: true };
		try {
			const res = await fetch(`/api/devices/${device.id}/${action}`, { method: 'POST' });
			if (!res.ok) throw new Error(`Failed to ${action}`);
			const result = await res.json();
			if (result.lock_state) {
				devices = devices.map((d) =>
					d.id === device.id ? { ...d, lock_state: result.lock_state } : d
				);
			} else {
				pendingAction = { ...pendingAction, [device.id]: action === 'lock' ? 'locking' : 'unlocking' };
			}
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

	async function loadSentinels() {
		try {
			const res = await fetch('/api/sentinel/sentinels');
			if (res.ok) sentinels = await res.json();
		} catch {
			// ignore
		}
	}

	async function requestBrowserNotifications() {
		if (typeof Notification === 'undefined') return;
		const result = await Notification.requestPermission();
		browserNotifications = result === 'granted';
	}

	async function loadNotificationPrefs() {
		try {
			const res = await fetch('/api/notifications');
			if (res.ok) {
				const data = await res.json();
				emailNotifications = data.email;
			}
		} catch {
			// ignore
		}
	}

	async function toggleEmailNotifications() {
		const newValue = !emailNotifications;
		try {
			const res = await fetch('/api/notifications', {
				method: 'PUT',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ email: newValue })
			});
			if (res.ok) {
				emailNotifications = newValue;
			}
		} catch {
			// ignore
		}
	}

	function urlBase64ToUint8Array(base64String: string): Uint8Array {
		const padding = '='.repeat((4 - (base64String.length % 4)) % 4);
		const base64 = (base64String + padding).replace(/-/g, '+').replace(/_/g, '/');
		const raw = atob(base64);
		const arr = new Uint8Array(raw.length);
		for (let i = 0; i < raw.length; i++) arr[i] = raw.charCodeAt(i);
		return arr;
	}

	let pushSupported = $state(
		typeof navigator !== 'undefined' &&
			'serviceWorker' in navigator &&
			'PushManager' in window
	);

	async function getSwRegistration(): Promise<ServiceWorkerRegistration | null> {
		if (!pushSupported) return null;
		try {
			return await navigator.serviceWorker.register('/sw.js', { updateViaCache: 'none' });
		} catch {
			return null;
		}
	}

	async function initPushSubscription() {
		const reg = await getSwRegistration();
		if (!reg) return;
		try {
			const sub = await reg.pushManager.getSubscription();
			pushNotifications = sub !== null;
		} catch {
			// ignore
		}
	}

	async function togglePushNotifications() {
		const reg = await getSwRegistration();
		if (!reg) return;
		pushLoading = true;
		try {
			await navigator.serviceWorker.ready;

			if (pushNotifications) {
				// Unsubscribe
				const sub = await reg.pushManager.getSubscription();
				if (sub) {
					await fetch('/api/push/unsubscribe', {
						method: 'POST',
						headers: { 'Content-Type': 'application/json' },
						body: JSON.stringify({ endpoint: sub.endpoint })
					});
					await sub.unsubscribe();
				}
				pushNotifications = false;
			} else {
				// Subscribe
				const res = await fetch('/api/push/vapid-key');
				if (!res.ok) return;
				const { key } = await res.json();
				const sub = await reg.pushManager.subscribe({
					userVisibleOnly: true,
					applicationServerKey: urlBase64ToUint8Array(key)
				});
				const subJson = sub.toJSON();
				await fetch('/api/push/subscribe', {
					method: 'POST',
					headers: { 'Content-Type': 'application/json' },
					body: JSON.stringify({
						endpoint: sub.endpoint,
						p256dh: subJson.keys?.p256dh ?? '',
						auth: subJson.keys?.auth ?? ''
					})
				});
				pushNotifications = true;
			}
		} catch {
			// ignore
		} finally {
			pushLoading = false;
		}
	}

	function fireBrowserNotification(title: string, body: string) {
		if (typeof Notification === 'undefined') return;
		if (Notification.permission !== 'granted') return;
		new Notification(title, { body, icon: '/favicon.png' });
	}

	function formatDate(iso: string): string {
		return new Date(iso).toLocaleDateString(undefined, {
			month: 'short',
			day: 'numeric',
			hour: '2-digit',
			minute: '2-digit'
		});
	}

	// ── WebSocket live updates ────────────────────────────────────────────

	let ws: WebSocket | null = null;
	let wsConnected = $state(false);
	let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

	function connectWebSocket() {
		if (ws) return;

		const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
		const url = `${proto}//${location.host}/api/ws`;

		const socket = new WebSocket(url);

		socket.addEventListener('open', () => {
			ws = socket;
			wsConnected = true;
		});

		socket.addEventListener('message', (ev) => {
			try {
				handleWsMessage(JSON.parse(ev.data));
			} catch {
				// ignore malformed messages
			}
		});

		socket.addEventListener('close', () => {
			ws = null;
			wsConnected = false;
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
			case 'scan': {
				const scanAction = msg.data.action as string;
				const scanTagId = msg.data.tag_id as string;
				scanLog = [
					{
						id: crypto.randomUUID(),
						tag_id: scanTagId,
						action: scanAction,
						created_at: msg.data.created_at as string
					},
					...scanLog
				];
				fireBrowserNotification(
					`Access ${scanAction === 'granted' ? 'Granted' : 'Denied'}`,
					`Card ${scanTagId} — ${scanAction}`
				);
				break;
			}
			case 'mode_changed':
				sentinelMode = msg.data.mode as string;
				break;
			case 'card_added': {
				const newCard = {
					id: msg.data.id as string,
					tag_id: msg.data.tag_id as string,
					label: (msg.data.label as string | null) ?? null,
					created_at: msg.data.created_at as string
				};
				if (!cards.some((c) => c.id === newCard.id)) {
					cards = [newCard, ...cards];
				}
				break;
			}
			case 'card_removed':
				cards = cards.filter((c) => c.id !== (msg.data.id as string));
				break;
			case 'lock_state': {
				const lsDeviceId = msg.data.device_id as string;
				const lsState = msg.data.lock_state as string;
				devices = devices.map((d) =>
					d.id === lsDeviceId ? { ...d, lock_state: lsState } : d
				);
				const { [lsDeviceId]: _, ...remainingPending } = pendingAction;
				pendingAction = remainingPending;
				const lsDevice = devices.find((d) => d.id === lsDeviceId);
				fireBrowserNotification(
					`Lock ${lsState}`,
					`${lsDevice?.name ?? lsDeviceId} is now ${lsState}`
				);
				break;
			}
			case 'sentinel_connected': {
				const scId = msg.data.id as string;
				const scName = msg.data.name as string;
				const existing = sentinels.find((s) => s.id === scId);
				if (existing) {
					sentinels = sentinels.map((s) =>
						s.id === scId ? { ...s, connected: true, last_connected_at: new Date().toISOString() } : s
					);
				} else {
					sentinels = [...sentinels, {
						id: scId,
						name: scName,
						connected: true,
						last_connected_at: new Date().toISOString(),
						created_at: new Date().toISOString()
					}];
				}
				break;
			}
			case 'sentinel_disconnected': {
				const sdId = msg.data.id as string;
				sentinels = sentinels.map((s) =>
					s.id === sdId ? { ...s, connected: false } : s
				);
				break;
			}
		}
	}

	async function loadCurrentUser() {
		try {
			const res = await fetch('/api/auth/me');
			if (res.ok) {
				const data = await res.json();
				currentUserEmail = data.email;
			}
		} catch {
			// ignore
		}
	}

	async function loadPendingUsers() {
		try {
			const res = await fetch('/api/admin/pending-users');
			if (res.ok) {
				pendingUsers = await res.json();
			} else if (res.status === 401 || res.status === 403) {
				pendingUsers = [];
			} else {
				pendingUsersError = 'Failed to load pending users';
			}
		} catch {
			pendingUsersError = 'Failed to load pending users';
		}
	}

	async function approveUser(id: string) {
		pendingUsersError = null;
		const prev = pendingUsers;
		pendingUsers = pendingUsers.filter((u) => u.id !== id);
		try {
			const res = await fetch(`/api/admin/users/${id}/approve`, { method: 'POST' });
			if (!res.ok) {
				pendingUsers = prev;
				if (res.status !== 403) pendingUsersError = 'Failed to approve user';
			}
		} catch {
			pendingUsers = prev;
			pendingUsersError = 'Failed to approve user';
		}
	}

	async function deletePendingUser(id: string) {
		pendingUsersError = null;
		const prev = pendingUsers;
		pendingUsers = pendingUsers.filter((u) => u.id !== id);
		try {
			const res = await fetch(`/api/admin/users/${id}`, { method: 'DELETE' });
			if (!res.ok) {
				pendingUsers = prev;
				if (res.status !== 403) pendingUsersError = 'Failed to delete user';
			}
		} catch {
			pendingUsers = prev;
			pendingUsersError = 'Failed to delete user';
		}
	}

	$effect(() => {
		checkUtec();
		loadCurrentUser();
		loadPendingUsers();
		loadSentinelMode();
		loadCards();
		loadScanLog();
		loadSentinels();
		loadNotificationPrefs();
		initPushSubscription();
		connectWebSocket();

		return () => {
			if (reconnectTimer) clearTimeout(reconnectTimer);
			if (ws) ws.close();
		};
	});

	$effect(() => {
		if (isUtecAuthenticated) {
			loadDevices().then((loaded) => {
				for (const d of loaded) {
					loadLockUsers(d.id);
				}
			});
		}
	});
</script>

<svelte:head>
	<title>Panopticon</title>
</svelte:head>

<main class="flex flex-1 justify-center p-6 lg:pt-10">
	<div class="w-full max-w-5xl space-y-6">
		<!-- Header -->
		<div class="flex items-center justify-between">
			<div class="flex items-center gap-3">
				<h1 class="h2">Panopticon</h1>
				<span
					class="inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium {wsConnected
						? 'bg-success-500/15 text-success-400'
						: 'bg-surface-700 text-surface-400 animate-pulse'}"
				>
					<span
						class="h-1.5 w-1.5 rounded-full {wsConnected
							? 'bg-success-500'
							: 'bg-surface-500'}"
					></span>
					{wsConnected ? 'Connected' : 'Reconnecting'}
				</span>
			</div>
			<div class="flex items-center gap-3">
				{#if currentUserEmail}
					<span class="text-sm text-surface-400">{currentUserEmail}</span>
				{/if}
				<button
					class="text-sm text-surface-500 hover:text-surface-300 cursor-pointer"
					onclick={handleLogout}
				>
					Sign out
				</button>
			</div>
		</div>

		<!-- Pending Users (admin only, shown when non-empty) -->
		{#if pendingUsers.length > 0 || pendingUsersError}
			<div class="card preset-filled-surface-900 space-y-4 p-6">
				<h2 class="h5">Pending Users</h2>
				{#if pendingUsersError}
					<p class="text-sm text-error-400">{pendingUsersError}</p>
				{/if}
				<div class="space-y-2">
					{#each pendingUsers as pu (pu.id)}
						<div class="flex items-center justify-between rounded-md bg-surface-800 px-3 py-2">
							<div class="flex items-center gap-3 min-w-0 flex-1">
								<div class="min-w-0">
									<p class="text-sm text-surface-200 truncate">{pu.email}</p>
									<p class="text-xs text-surface-500">{formatDate(pu.created_at)}</p>
								</div>
								<span
									class="flex-shrink-0 rounded-full px-2 py-0.5 text-xs font-medium {pu.email_confirmed
										? 'bg-success-500/15 text-success-400'
										: 'bg-warning-500/15 text-warning-400'}"
								>
									{pu.email_confirmed ? 'Confirmed' : 'Unconfirmed'}
								</span>
							</div>
							<div class="flex items-center gap-2 flex-shrink-0 ml-3">
								<button
									class="btn btn-sm preset-outlined-success-500"
									onclick={() => approveUser(pu.id)}
								>
									Approve
								</button>
								<button
									class="btn btn-sm preset-outlined-error-500"
									onclick={() => deletePendingUser(pu.id)}
								>
									Delete
								</button>
							</div>
						</div>
					{/each}
				</div>
			</div>
		{/if}

		<!-- Two-column grid on desktop, single column on mobile -->
		<div class="grid grid-cols-1 lg:grid-cols-2 gap-6">
			<!-- Left column: U-Tec + Devices + Access Control -->
			<div class="space-y-6">
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
						<div class="card preset-filled-surface-900 space-y-4 p-6 animate-pulse">
							<!-- Name + status dot -->
							<div class="flex items-center justify-between">
								<div class="placeholder h-5 w-32 rounded"></div>
								<div class="placeholder-circle h-2 w-2"></div>
							</div>
							<!-- Lock icon + state + battery -->
							<div class="flex items-center justify-between">
								<div class="flex items-center gap-3">
									<div class="placeholder-circle h-10 w-10"></div>
									<div class="placeholder h-4 w-16 rounded"></div>
								</div>
								<div class="placeholder h-4 w-10 rounded"></div>
							</div>
							<!-- Button -->
							<div class="placeholder h-10 w-full rounded-lg"></div>
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
										{#if pendingAction[device.id]}
											<div
												class="flex h-10 w-10 items-center justify-center rounded-full bg-primary-500/15"
											>
												<svg
													class="h-5 w-5 text-primary-400 animate-spin"
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
											<span class="text-sm font-medium text-primary-400 animate-pulse">
												{pendingAction[device.id] === 'locking' ? 'Locking...' : 'Unlocking...'}
											</span>
										{:else if device.lock_state === 'locked'}
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
									disabled={actionInFlight[device.id] || pendingAction[device.id] || !device.online}
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

								<!-- Lock Users -->
								{#if lockUsersLoading[device.id]}
									<div class="border-t border-surface-700 pt-3">
										<p class="text-xs text-surface-500 animate-pulse">Loading users...</p>
									</div>
								{:else if lockUsers[device.id] != null}
									<div class="border-t border-surface-700 pt-3 space-y-2">
										<h4 class="text-xs font-medium text-surface-400 uppercase tracking-wide">Lock Users</h4>
										{#if lockUsers[device.id].length === 0}
											<p class="text-xs text-surface-500">No users configured.</p>
										{:else}
											{#each lockUsers[device.id] as user (user.id)}
												<div class="flex items-center justify-between rounded-md bg-surface-800 px-3 py-2">
													<span class="text-sm text-surface-200">{user.name}</span>
													<!-- U-Tec user types: 1 = Admin, 3 = User (see bug #6 in README) -->
													<span class="text-xs text-surface-500">
														{user.type === 1 ? 'Admin' : user.type === 3 ? 'User' : `Type ${user.type}`}
													</span>
												</div>
											{/each}
										{/if}
									</div>
								{/if}
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

				<!-- Notifications -->
				<div class="card preset-filled-surface-900 space-y-4 p-6">
					<h2 class="h5">Notifications</h2>
					<div class="space-y-3">
						<!-- Browser notifications toggle -->
						<div class="flex items-center justify-between">
							<div>
								<p class="text-sm text-surface-200">Browser notifications</p>
								<p class="text-xs text-surface-500">
									{#if typeof Notification === 'undefined'}
										Not supported in this browser
									{:else if browserNotifications}
										Enabled
									{:else}
										Click to enable desktop alerts
									{/if}
								</p>
							</div>
							<button
								class="btn btn-sm {browserNotifications
									? 'preset-filled-primary-500'
									: 'preset-outlined-surface-500'}"
								disabled={typeof Notification === 'undefined' ||
									Notification.permission === 'denied'}
								onclick={requestBrowserNotifications}
							>
								{browserNotifications ? 'On' : 'Off'}
							</button>
						</div>
						<!-- Push notifications toggle -->
						{#if pushSupported}
							<div class="flex items-center justify-between">
								<div>
									<p class="text-sm text-surface-200">Push notifications</p>
									<p class="text-xs text-surface-500">
										{#if pushNotifications}
											Enabled — works even when tab is closed
										{:else}
											Receive alerts when the browser is closed
										{/if}
									</p>
								</div>
								<button
									class="btn btn-sm {pushNotifications
										? 'preset-filled-primary-500'
										: 'preset-outlined-surface-500'}"
									disabled={pushLoading}
									onclick={togglePushNotifications}
								>
									{#if pushLoading}
										<span class="animate-pulse">...</span>
									{:else}
										{pushNotifications ? 'On' : 'Off'}
									{/if}
								</button>
							</div>
						{/if}
						<!-- Email notifications toggle -->
						<div class="flex items-center justify-between">
							<div>
								<p class="text-sm text-surface-200">Email notifications</p>
								<p class="text-xs text-surface-500">
									Receive an email on every access event
								</p>
							</div>
							<button
								class="btn btn-sm {emailNotifications
									? 'preset-filled-primary-500'
									: 'preset-outlined-surface-500'}"
								onclick={toggleEmailNotifications}
							>
								{emailNotifications ? 'On' : 'Off'}
							</button>
						</div>
					</div>
				</div>
			</div>

			<!-- Right column: Sentinels + Enrolled Cards + Recent Scans -->
			<div class="space-y-6">
				<!-- Sentinels -->
				<div class="card preset-filled-surface-900 space-y-4 p-6">
					<h2 class="h5">Sentinels</h2>
					{#if sentinels.length === 0}
						<p class="text-sm text-surface-400">No sentinels registered yet.</p>
					{:else}
						<div class="space-y-2">
							{#each sentinels as s (s.id)}
								<a
									href={`/sentinel/${s.id}`}
									class="flex items-center justify-between rounded-md bg-surface-800 px-3 py-2 hover:bg-surface-700 transition-colors"
								>
									<div class="flex items-center gap-3">
										<div
											class="h-2 w-2 rounded-full {s.connected
												? 'bg-success-500'
												: 'bg-surface-600'}"
										></div>
										<div>
											<p class="text-sm text-surface-200">{s.name}</p>
											{#if s.last_connected_at}
												<p class="text-xs text-surface-500">{formatDate(s.last_connected_at)}</p>
											{/if}
										</div>
									</div>
									<span class="text-xs text-surface-500">{s.connected ? 'Connected' : 'Disconnected'}</span>
								</a>
							{/each}
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
		</div>
	</div>
</main>
