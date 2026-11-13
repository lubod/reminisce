import { WebTracerProvider } from '@opentelemetry/sdk-trace-web';
import { BatchSpanProcessor } from '@opentelemetry/sdk-trace-base';
import { OTLPTraceExporter } from '@opentelemetry/exporter-trace-otlp-http';
import { ParentBasedSampler, TraceIdRatioBasedSampler } from '@opentelemetry/sdk-trace-base';
import { ZoneContextManager } from '@opentelemetry/context-zone';
import { registerInstrumentations } from '@opentelemetry/instrumentation';
import { XMLHttpRequestInstrumentation } from '@opentelemetry/instrumentation-xml-http-request';
import { DocumentLoadInstrumentation } from '@opentelemetry/instrumentation-document-load';
import { UserInteractionInstrumentation } from '@opentelemetry/instrumentation-user-interaction';
import { Resource } from '@opentelemetry/resources';
import { ATTR_SERVICE_NAME, ATTR_SERVICE_VERSION } from '@opentelemetry/semantic-conventions';

import { logger } from './utils/logger';

const setupTelemetry = () => {
    // Only enable if explicitly enabled via environment variable
    if (import.meta.env.VITE_ENABLE_OTEL !== "true" && !import.meta.env.VITE_OTEL_EXPORTER_OTLP_ENDPOINT) {
        return;
    }
    const resource = new Resource({
        [ATTR_SERVICE_NAME]: 'reminisce-frontend',
        [ATTR_SERVICE_VERSION]: import.meta.env.VITE_APP_VERSION || '0.1.0',
        'deployment.environment.name': import.meta.env.VITE_ENVIRONMENT || 'development',
    });

    // Sampling configuration - default to 100% in development, configurable for production
    const sampleRate = parseFloat(import.meta.env.VITE_OTEL_TRACE_SAMPLE_RATE || '1.0');
    const sampler = new ParentBasedSampler({
        root: new TraceIdRatioBasedSampler(sampleRate),
    });

    // OTLP endpoint is configurable via environment variable
    // Default: localhost:4318 for local development
    // Production: should be set to the tempo endpoint accessible from the browser
    const collectorOptions = {
        url: import.meta.env.VITE_OTEL_EXPORTER_OTLP_ENDPOINT || 'http://localhost:4318/v1/traces',
    };

    const exporter = new OTLPTraceExporter(collectorOptions);

    // BatchSpanProcessor is better for performance than SimpleSpanProcessor
    const provider = new WebTracerProvider({
        resource: resource,
        sampler: sampler,
        spanProcessors: [new BatchSpanProcessor(exporter)],
    });

    provider.register({
        contextManager: new ZoneContextManager(),
    });

    registerInstrumentations({
        instrumentations: [
            new DocumentLoadInstrumentation(),
            new UserInteractionInstrumentation({
                eventNames: ['click', 'keypress', 'submit'],
            }),
            new XMLHttpRequestInstrumentation({
                propagateTraceHeaderCorsUrls: [
                    /^\/api\//,
                    /localhost:8080/,
                    /reminisce\.local/,
                ],
                clearTimingResources: true,
            }),
        ],
    });

    logger.info('OpenTelemetry initialized');
};

export default setupTelemetry;
