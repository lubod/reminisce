import { WebTracerProvider } from '@opentelemetry/sdk-trace-web';
import { BatchSpanProcessor } from '@opentelemetry/sdk-trace-base';
import { OTLPTraceExporter } from '@opentelemetry/exporter-trace-otlp-http';
import { ParentBasedSampler, TraceIdRatioBasedSampler } from '@opentelemetry/sdk-trace-base';
import { ZoneContextManager } from '@opentelemetry/context-zone';
import { registerInstrumentations } from '@opentelemetry/instrumentation';
import { FetchInstrumentation } from '@opentelemetry/instrumentation-fetch';
import { DocumentLoadInstrumentation } from '@opentelemetry/instrumentation-document-load';
import { UserInteractionInstrumentation } from '@opentelemetry/instrumentation-user-interaction';
import { Resource } from '@opentelemetry/resources';
import { SemanticResourceAttributes } from '@opentelemetry/semantic-conventions';

const setupTelemetry = () => {
    const resource = new Resource({
        [SemanticResourceAttributes.SERVICE_NAME]: 'reminisce-frontend',
        [SemanticResourceAttributes.SERVICE_VERSION]: import.meta.env.VITE_APP_VERSION || '0.1.0',
        'deployment.environment': import.meta.env.VITE_ENVIRONMENT || 'development',
    });

    // Sampling configuration - default to 100% in development, configurable for production
    const sampleRate = parseFloat(import.meta.env.VITE_OTEL_TRACE_SAMPLE_RATE || '1.0');
    const sampler = new ParentBasedSampler({
        root: new TraceIdRatioBasedSampler(sampleRate),
    });

    const provider = new WebTracerProvider({
        resource: resource,
        sampler: sampler,
    });

    // OTLP endpoint is configurable via environment variable
    // Default: localhost:4318 for local development
    // Production: should be set to the tempo endpoint accessible from the browser
    const collectorOptions = {
        url: import.meta.env.VITE_OTEL_EXPORTER_OTLP_ENDPOINT || 'http://localhost:4318/v1/traces',
    };

    const exporter = new OTLPTraceExporter(collectorOptions);

    // BatchSpanProcessor is better for performance than SimpleSpanProcessor
    provider.addSpanProcessor(new BatchSpanProcessor(exporter));

    provider.register({
        contextManager: new ZoneContextManager(),
    });

    registerInstrumentations({
        instrumentations: [
            new DocumentLoadInstrumentation(),
            new UserInteractionInstrumentation({
                eventNames: ['click', 'keypress', 'submit'],
            }),
            new FetchInstrumentation({
                propagateTraceHeaderCorsUrls: [
                    /localhost:8080/, // Backend URL
                    /reminisce\.local/, // If you use a domain
                ],
                clearTimingResources: true,
            }),
        ],
    });

    console.log('OpenTelemetry initialized');
};

export default setupTelemetry;
