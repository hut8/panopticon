<script lang="ts">
	import { page } from '$app/stores';

	let password = $state('');
	let confirmPassword = $state('');
	let error: string | null = $state(null);
	let success = $state(false);
	let loading = $state(false);

	let token = $derived(new URLSearchParams($page.url.search).get('token') || '');

	async function handleSubmit(e: Event) {
		e.preventDefault();
		error = null;

		if (password !== confirmPassword) {
			error = 'Passwords do not match';
			return;
		}

		if (!token) {
			error = 'Missing reset token';
			return;
		}

		loading = true;

		try {
			const res = await fetch('/api/auth/reset-password', {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ token, password })
			});

			const data = await res.json();

			if (!res.ok) {
				error = data.error || 'Password reset failed';
				return;
			}

			success = true;
		} catch {
			error = 'Network error. Please try again.';
		} finally {
			loading = false;
		}
	}
</script>

<svelte:head>
	<title>Reset Password — Panopticon</title>
</svelte:head>

<main class="flex flex-1 items-center justify-center p-6">
	<div class="w-full max-w-sm space-y-6">
		<div class="text-center">
			<h1 class="h2">Panopticon</h1>
			<p class="mt-2 text-sm text-surface-400">Set a new password</p>
		</div>

		<div class="card preset-filled-surface-900 space-y-5 p-6">
			{#if success}
				<div class="space-y-4 text-center">
					<div class="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-success-500/15">
						<svg class="h-7 w-7 text-success-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
							<path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
						</svg>
					</div>
					<p class="text-sm text-surface-300">Your password has been reset.</p>
					<a href="/login" class="btn btn-base preset-filled-primary-500 w-full">Sign In</a>
				</div>
			{:else if !token}
				<div class="space-y-4 text-center">
					<p class="text-sm text-surface-400">Invalid or missing reset link.</p>
					<a href="/forgot-password" class="btn btn-base preset-outlined-surface-500 w-full">
						Request a new link
					</a>
				</div>
			{:else}
				{#if error}
					<div class="rounded-md bg-error-500/10 px-4 py-3 text-sm text-error-400">
						{error}
					</div>
				{/if}

				<form onsubmit={handleSubmit} class="space-y-4">
					<label class="label space-y-2">
						<span class="label-text">New Password</span>
						<input
							type="password"
							bind:value={password}
							required
							minlength="8"
							class="input preset-filled-surface-800 border border-surface-700 px-4 py-2.5"
							placeholder="At least 8 characters"
						/>
					</label>

					<label class="label space-y-2">
						<span class="label-text">Confirm Password</span>
						<input
							type="password"
							bind:value={confirmPassword}
							required
							minlength="8"
							class="input preset-filled-surface-800 border border-surface-700 px-4 py-2.5"
							placeholder="••••••••"
						/>
					</label>

					<button type="submit" class="btn btn-base preset-filled-primary-500 w-full" disabled={loading}>
						{loading ? 'Resetting...' : 'Reset Password'}
					</button>
				</form>
			{/if}
		</div>
	</div>
</main>
