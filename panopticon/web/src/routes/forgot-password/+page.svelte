<script lang="ts">
	let email = $state('');
	let error: string | null = $state(null);
	let sent = $state(false);
	let loading = $state(false);

	async function handleSubmit(e: Event) {
		e.preventDefault();
		error = null;
		loading = true;

		try {
			const res = await fetch('/api/auth/forgot-password', {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ email })
			});

			if (!res.ok) {
				const data = await res.json();
				error = data.error || 'Request failed';
				return;
			}

			sent = true;
		} catch {
			error = 'Network error. Please try again.';
		} finally {
			loading = false;
		}
	}
</script>

<svelte:head>
	<title>Forgot Password — Panopticon</title>
</svelte:head>

<main class="flex flex-1 items-center justify-center p-6">
	<div class="w-full max-w-sm space-y-6">
		<div class="text-center">
			<h1 class="h2">Panopticon</h1>
			<p class="mt-2 text-sm text-surface-400">Reset your password</p>
		</div>

		<div class="card preset-filled-surface-900 space-y-5 p-6">
			{#if sent}
				<div class="space-y-4 text-center">
					<div class="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-success-500/15">
						<svg class="h-7 w-7 text-success-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
							<path stroke-linecap="round" stroke-linejoin="round" d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
						</svg>
					</div>
					<p class="text-sm text-surface-300">
						If an account with that email exists, we've sent a password reset link.
					</p>
					<a href="/login" class="btn btn-base preset-outlined-surface-500 w-full">Back to Sign In</a>
				</div>
			{:else}
				{#if error}
					<div class="rounded-md bg-error-500/10 px-4 py-3 text-sm text-error-400">
						{error}
					</div>
				{/if}

				<p class="text-center text-sm text-surface-400">
					Enter your email and we'll send a link to reset your password.
				</p>

				<form onsubmit={handleSubmit} class="space-y-4">
					<label class="label space-y-2">
						<span class="label-text">Email</span>
						<input
							type="email"
							bind:value={email}
							required
							class="input preset-filled-surface-800 border border-surface-700 px-4 py-2.5"
							placeholder="you@example.com"
						/>
					</label>

					<button type="submit" class="btn btn-base preset-filled-primary-500 w-full" disabled={loading}>
						{loading ? 'Sending...' : 'Send Reset Link'}
					</button>
				</form>
			{/if}
		</div>

		{#if !sent}
			<p class="text-center text-sm text-surface-500">
				<a href="/login" class="anchor">Back to Sign In</a>
			</p>
		{/if}
	</div>
</main>
