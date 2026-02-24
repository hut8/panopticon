<script lang="ts">
	interface UtecStatus {
		authenticated: boolean;
		user_name: string | null;
		expires_at: string | null;
	}

	let utecStatus: UtecStatus | null = $state(null);
	let loading = $state(true);

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

	async function disconnectUtec() {
		try {
			await fetch('/auth/logout', { method: 'DELETE' });
			utecStatus = { authenticated: false, user_name: null, expires_at: null };
		} catch {
			// ignore
		}
	}

	async function handleLogout() {
		await fetch('/api/auth/logout', { method: 'POST' });
		window.location.href = '/login';
	}

	$effect(() => {
		checkUtec();
	});
</script>

<svelte:head>
	<title>Panopticon</title>
</svelte:head>

<main class="flex flex-1 items-center justify-center p-6">
	<div class="w-full max-w-sm space-y-6">
		<div class="flex items-center justify-between">
			<h1 class="h2">Panopticon</h1>
			<button class="text-sm text-surface-500 hover:text-surface-300 cursor-pointer" onclick={handleLogout}>
				Sign out
			</button>
		</div>

		<div class="card preset-filled-surface-900 space-y-5 p-6">
			<h2 class="h5">U-Tec Smart Lock</h2>

			{#if loading}
				<p class="text-sm text-surface-400 animate-pulse">Checking connection...</p>
			{:else if utecStatus?.authenticated}
				<div class="space-y-3">
					<div class="flex items-center gap-3 rounded-md bg-success-500/10 px-4 py-3">
						<div class="h-2 w-2 rounded-full bg-success-500"></div>
						<div>
							<p class="text-sm font-medium text-surface-200">Connected</p>
							<p class="text-xs text-surface-400">{utecStatus.user_name ?? 'Unknown user'}</p>
						</div>
					</div>
					{#if utecStatus.expires_at}
						<p class="text-xs text-surface-500">
							Token expires {new Date(utecStatus.expires_at).toLocaleString()}
						</p>
					{/if}
					<button class="btn btn-base preset-outlined-surface-500 w-full" onclick={disconnectUtec}>
						Disconnect
					</button>
				</div>
			{:else}
				<p class="text-sm text-surface-400">
					Connect your U-Tec account to manage your smart locks.
				</p>
				<a href="/auth/login" class="btn btn-base preset-filled-primary-500 w-full">
					Connect U-Tec Account
				</a>
			{/if}
		</div>
	</div>
</main>
