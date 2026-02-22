<script lang="ts">
	interface AuthStatus {
		authenticated: boolean;
		user_name: string | null;
		expires_at: string | null;
	}

	let status: AuthStatus | null = $state(null);
	let loading = $state(true);
	let error: string | null = $state(null);

	async function checkAuth() {
		try {
			const res = await fetch('/auth/status');
			status = await res.json();
		} catch (e) {
			error = 'Failed to check authentication status';
		} finally {
			loading = false;
		}
	}

	async function logout() {
		try {
			await fetch('/auth/logout', { method: 'DELETE' });
			status = { authenticated: false, user_name: null, expires_at: null };
		} catch (e) {
			error = 'Failed to logout';
		}
	}

	$effect(() => {
		checkAuth();
	});
</script>

<svelte:head>
	<title>Panopticon</title>
</svelte:head>

<div class="flex h-full items-center justify-center p-4">
	{#if loading}
		<div class="card preset-filled-surface-200-800 p-8 text-center">
			<p class="text-surface-600-400">Checking authentication...</p>
		</div>
	{:else if status?.authenticated}
		<div class="card preset-filled-surface-200-800 space-y-6 p-8 text-center">
			<h1 class="h2">Panopticon</h1>
			<p>
				Signed in as <strong>{status.user_name ?? 'Unknown'}</strong>
			</p>
			{#if status.expires_at}
				<p class="text-sm text-surface-600-400">
					Token expires {new Date(status.expires_at).toLocaleString()}
				</p>
			{/if}
			<button class="btn preset-filled-surface-400-600" onclick={logout}>
				Sign out
			</button>
		</div>
	{:else}
		<div class="card preset-filled-surface-200-800 space-y-6 p-8 text-center">
			<h1 class="h2">Panopticon</h1>
			<p class="text-surface-600-400">Sign in with your U-Tec account to manage your locks.</p>
			{#if error}
				<p class="preset-filled-error-500 rounded p-2 text-sm">{error}</p>
			{/if}
			<a href="/auth/login" class="btn preset-filled-primary-500 btn-lg">
				Sign in with U-Tec
			</a>
		</div>
	{/if}
</div>
